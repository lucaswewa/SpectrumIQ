"""Runs live view for a while with both previews streaming, and checks that no frame was lost.

Start the service first, with the simulated instrument at full size and rate:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml

then, from the repository:

    python examples/python/live_soak.py [http://127.0.0.1:5000] [seconds, 600 by default]

It starts live view, reads the overview and ROI MJPEG streams throughout, prints the camera's
statistics each minute, then stops live view. It succeeds if the camera lost no frame and both
streams kept coming. This is M2's second exit criterion. It needs only the standard library.
"""

import json
import sys
import threading
import time
import urllib.request

ROOT = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5000").rstrip("/")
SECONDS = float(sys.argv[2]) if len(sys.argv) > 2 else 600.0
API = ROOT + "/api/v1"


def call(method, url, body=None):
    """Sends a request, and gives back the decoded JSON answer."""
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request) as answer:
        return json.loads(answer.read() or b"null")


def invoke(thing, action):
    """Invokes an action without inputs, and waits for it to end."""
    invocation = call("POST", f"{API}/{thing}/{action}")
    while invocation["status"] in ("pending", "running"):
        time.sleep(0.05)
        invocation = call("GET", invocation["href"])
    if invocation["status"] != "completed":
        raise RuntimeError(f"{thing}.{action}: {invocation}")


def watch(stream, counts, until):
    """Counts the JPEGs of an MJPEG stream until `until`."""
    with urllib.request.urlopen(f"{API}/camera/{stream}") as body:
        data = b""
        while time.monotonic() < until:
            data += body.read1(1 << 20)
            end = data.rfind(b"\xff\xd9")
            if end >= 0:
                counts[stream] += data[: end + 2].count(b"\xff\xd9")
                data = data[end + 2 :]


def main():
    print("health:", call("GET", f"{ROOT}/api/siq/system/health"))
    invoke("camera", "start_live")
    started = time.monotonic()
    until = started + SECONDS
    counts = {"overview": 0, "roi": 0}
    watchers = [threading.Thread(target=watch, args=(name, counts, until)) for name in counts]
    for watcher in watchers:
        watcher.start()
    while time.monotonic() < until:
        time.sleep(min(60.0, max(0.0, until - time.monotonic())))
        stats = call("GET", f"{API}/camera/stats")
        print(f"{time.monotonic() - started:5.0f} s: {stats['published']} published at {stats['fps']:.2f} fps,"
              f" lost {stats['lost_no_slot']} for a slot and {stats['lost_device']} by the camera;"
              f" previews {counts['overview']} overview, {counts['roi']} ROI")
    for watcher in watchers:
        watcher.join()
    invoke("camera", "stop_live")
    stats = call("GET", f"{API}/camera/stats")
    lost = stats["lost_no_slot"] + stats["lost_device"]
    streamed = all(count > 0 for count in counts.values())
    print(f"{stats['published']} frames in {SECONDS:.0f} s, {lost} lost; {counts}")
    return 0 if lost == 0 and streamed else 1


if __name__ == "__main__":
    sys.exit(main())
