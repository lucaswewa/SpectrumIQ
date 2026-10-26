"""Runs a tile scan through the `workflow` Thing, and follows its progress.

Start the service first, with the simulated instrument:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml

then, from the repository:

    python examples/python/tile_scan.py [http://127.0.0.1:5000]

A tile scan needs the objective's stage-camera calibration: without one, it calibrates first.
Then it covers a region of 2 x 2 frames about (-20, -15) mm at 10 % overlap in brightfield,
printing its progress while it runs, and where its dataset is. It needs only the standard
library.
"""

import json
import sys
import time
import urllib.request

ROOT = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5000").rstrip("/")
API = ROOT + "/api/v1"


def call(method, url, body=None):
    """Sends a request, and gives back the decoded JSON answer."""
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request) as answer:
        return json.loads(answer.read() or b"null")


def invoke(thing, action, inputs, watch=None):
    """Invokes an action, waits for it to end, calling `watch` meanwhile, and gives back its
    output."""
    invocation = call("POST", f"{API}/{thing}/{action}", inputs)
    href = invocation["href"]
    while invocation["status"] in ("pending", "running"):
        time.sleep(0.25)
        if watch:
            watch()
        invocation = call("GET", href)
    if invocation["status"] != "completed":
        raise RuntimeError(f"{thing}.{action}: {invocation}")
    return invocation.get("output")


def progress():
    """Prints how far the run has gone."""
    now = call("GET", f"{API}/workflow/progress")
    if now:
        eta = now["eta_s"]
        left = f", about {eta:.0f} s left" if eta is not None else ""
        print(f"  {now['done']} of {now['total']} steps{left}: {now['step']}")


def main():
    status = call("GET", f"{API}/calibration/status")
    if not any(objective["calibration"] for objective in status):
        print("calibrating the stage against the camera first")
        invoke("calibration", "stage_camera", {})
    sensor = call("GET", f"{API}/camera/sensor")
    # A frame's size on the stage, by the objective's 10x on the sensor's pixels.
    field = [sensor["width"] * sensor["pixel_size"] / 10, sensor["height"] * sensor["pixel_size"] / 10]
    step = [0.9 * field[0], 0.9 * field[1]]
    x, y = -20_000.0, -15_000.0
    region = {"shape": "rect", "x0": x - 0.8 * step[0], "y0": y - 0.8 * step[1],
              "x1": x + 0.8 * step[0], "y1": y + 0.8 * step[1]}
    scan = {"name": "tile-scan", "region": region, "overlap": 0.1, "channels": [1]}
    finished = invoke("workflow", "tile_scan", scan, watch=progress)
    print(f"{finished['outcome']}: {finished['frames']} frames in dataset {finished['dataset']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
