"""Moves the stage and saves an overview frame before and after the move, through the HTTP API.

Start the service first, with the simulated instrument:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml
    set SIQ_USER=<a user>& set SIQ_PASSWORD=<its password>   (siq_session.py)

then, from the repository:

    python examples/python/live_and_move.py [http://127.0.0.1:5000]

It starts live view, saves `before.jpg`, moves the stage 3 mm, saves `after.jpg`, and stops live
view. The frames differ, because the camera sees the new place. It needs only the standard
library.
"""

import json
import sys
import time
import urllib.request

import siq_session

ROOT = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5000").rstrip("/")
API = ROOT + "/api/v1"


def call(method, url, body=None):
    """Sends a request, and gives back the decoded JSON answer."""
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request) as answer:
        return json.loads(answer.read() or b"null")


def invoke(thing, action, **inputs):
    """Invokes an action, waits for it to end, and gives back its output."""
    invocation = call("POST", f"{API}/{thing}/{action}", inputs or None)
    href = invocation["href"]
    while invocation["status"] in ("pending", "running"):
        time.sleep(0.05)
        invocation = call("GET", href)
    if invocation["status"] != "completed":
        raise RuntimeError(f"{thing}.{action}: {invocation}")
    return invocation.get("output")


def frame(stream="overview"):
    """The next JPEG of an MJPEG stream."""
    with urllib.request.urlopen(f"{API}/camera/{stream}") as body:
        data = b""
        while True:
            data += body.read1(65536)
            start = data.find(b"\xff\xd8")
            end = data.find(b"\xff\xd9", start + 2)
            if start >= 0 and end > start:
                return data[start : end + 2]


def main():
    print("health:", call("GET", f"{ROOT}/api/siq/system/health"))
    invoke("camera", "start_live")
    time.sleep(1.0)
    before = frame()
    open("before.jpg", "wb").write(before)
    status = invoke("stage", "move_to", x=-3000.0, y=-2000.0, z=0.0)
    print("the stage is at", status["position"], status["state"])
    # Frames exposed before the stage stopped may still be on their way.
    time.sleep(1.0)
    after = frame()
    open("after.jpg", "wb").write(after)
    invoke("camera", "stop_live")
    changed = "differ" if before != after else "are the same"
    print(f"before.jpg ({len(before)} bytes) and after.jpg ({len(after)} bytes) {changed}")
    return 0 if before != after else 1


if __name__ == "__main__":
    siq_session.login(ROOT, control=True)
    sys.exit(main())
