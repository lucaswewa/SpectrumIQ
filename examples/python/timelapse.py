"""Runs a time-lapse at the saved positions through the `workflow` Thing, pausing it once.

Start the service first, with the simulated instrument:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml
    set SIQ_USER=<a user>& set SIQ_PASSWORD=<its password>   (siq_session.py)

then, from the repository:

    python examples/python/timelapse.py [http://127.0.0.1:5000]

With fewer than two positions saved, it saves two: where the stage is, and 1 mm along. Then every
10 s, four times, it visits them and takes a frame at each. It pauses the run after its first
point for 5 s, and prints each frame as the run reports it, `t` the point and `p` the position.
It needs only the standard library.
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


def invoke(thing, action, inputs):
    """Invokes an action, waits for it to end, and gives back its output."""
    invocation = call("POST", f"{API}/{thing}/{action}", inputs)
    while invocation["status"] in ("pending", "running"):
        time.sleep(0.05)
        invocation = call("GET", invocation["href"])
    if invocation["status"] != "completed":
        raise RuntimeError(f"{thing}.{action}: {invocation}")
    return invocation.get("output")


def main():
    positions = call("GET", f"{API}/workflow/positions")
    if len(positions) < 2:
        at = call("GET", f"{API}/stage/position")
        for name, dx in (("timelapse-1", 0.0), ("timelapse-2", -1000.0)):
            if all(p["name"] != name for p in positions):
                positions = invoke("workflow", "save_position", {"name": name, "x": at["x"] + dx, "y": at["y"]})
    print("positions: " + ", ".join(f"{p['name']} ({p['x']:.0f}, {p['y']:.0f})" for p in positions))
    # A time-lapse of a multi-position: the plan's own steps.
    visit = {"step": "multi_position", "positions": positions}
    lapse = {"name": "timelapse", "interval_s": 10.0, "count": 4, "steps": [visit]}
    invocation = call("POST", f"{API}/workflow/time_lapse", lapse)
    href, began, paused, seen = invocation["href"], time.monotonic(), False, None
    while invocation["status"] in ("pending", "running"):
        time.sleep(0.2)
        state = call("GET", f"{API}/workflow/state")
        now = call("GET", f"{API}/workflow/progress")
        if now and now["step"].startswith("snap") and now["step"] != seen and state == "running":
            seen = now["step"]
            print(f"{time.monotonic() - began:5.1f} s: {seen}")
        # After the first point: a quarter of the steps.
        if now and now["done"] == now["total"] // 4 and not paused:
            paused = True
            invoke("workflow", "pause", {})
            print(f"{time.monotonic() - began:5.1f} s: paused")
            time.sleep(5)
            invoke("workflow", "resume", {})
            print(f"{time.monotonic() - began:5.1f} s: resumed")
        invocation = call("GET", href)
    if invocation["status"] != "completed":
        raise RuntimeError(f"workflow.time_lapse: {invocation}")
    finished = invocation["output"]
    print(f"{finished['frames']} frames in dataset {finished['dataset']}: its layout.json has each point's times")
    return 0


if __name__ == "__main__":
    siq_session.login(ROOT, control=True)
    sys.exit(main())
