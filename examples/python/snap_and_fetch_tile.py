"""Snaps a frame into a dataset, waits for its pyramid, and fetches deep-zoom tiles of it, through
the HTTP API.

Start the service first, with the simulated instrument:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml

then, from the repository:

    python examples/python/snap_and_fetch_tile.py [http://127.0.0.1:5000] [channel id ...]

It snaps a frame in each channel given (in the channel in use if none is), which the service makes
a dataset: the frames recorded raw, written as level 0 of the OME-Zarr image `snap`, and its
pyramid built as a job. It then reads the plane's DZI descriptor and saves `tile.jpeg`, the tile
at the centre of the level that fits 1920 pixels across, and `thumb.jpg`, the dataset's thumbnail.
It prints how long each step took. It needs only the standard library.
"""

import json
import math
import re
import sys
import time
import urllib.request

ROOT = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5000").rstrip("/")
CHANNELS = [int(channel) for channel in sys.argv[2:]]
API = ROOT + "/api/v1"
DATA = ROOT + "/api/siq/data"


def call(method, url, body=None):
    """Sends a request, and gives back the decoded JSON answer."""
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request) as answer:
        return json.loads(answer.read() or b"null")


def fetch(url):
    """The bytes at `url`."""
    with urllib.request.urlopen(url) as answer:
        return answer.read()


def invoke(thing, action, inputs):
    """Invokes an action, waits for it to end, and gives back its output."""
    invocation = call("POST", f"{API}/{thing}/{action}", inputs)
    href = invocation["href"]
    while invocation["status"] in ("pending", "running"):
        time.sleep(0.05)
        invocation = call("GET", href)
    if invocation["status"] != "completed":
        raise RuntimeError(f"{thing}.{action}: {invocation}")
    return invocation.get("output")


def wait_for(dataset, kind):
    """Waits for the dataset's `kind` job to end, from its manifest's history."""
    while True:
        history = call("GET", f"{DATA}/{dataset}/manifest")["history"]
        states = [record["state"] for record in history if record["kind"] == kind]
        if states and states[-1] not in ("queued", "running"):
            return states[-1]
        time.sleep(0.1)


def main():
    started = time.perf_counter()
    inputs = {"channels": CHANNELS} if CHANNELS else {}
    snap = invoke("datasets", "snap_to_dataset", inputs)
    dataset, image = snap["dataset"], snap["image"]
    made = time.perf_counter()
    print(f"dataset {dataset['id']} ({dataset['name']}): {dataset['frames']} frame(s), image {image},"
          f" in {made - started:.2f} s")

    state = wait_for(dataset["id"], "pyramid")
    built = time.perf_counter()
    print(f"its pyramid: {state}, {built - made:.2f} s later")

    plane = f"{DATA}/{dataset['id']}/{image}/0/0/0"
    descriptor = fetch(f"{plane}.dzi").decode()
    width, height = (int(n) for n in re.search(r'Width="(\d+)" Height="(\d+)"', descriptor).groups())
    top = math.ceil(math.log2(max(width, height)))
    # The level that fits 1920 pixels across, and its tile at the centre.
    level = top - max(0, math.ceil(math.log2(width / 1920)))
    scale = 2 ** (top - level)
    col, row = (math.ceil(width / scale) // 2) // 254, (math.ceil(height / scale) // 2) // 254
    tile = fetch(f"{plane}_files/{level}/{col}_{row}.jpeg")
    shown = time.perf_counter()
    open("tile.jpeg", "wb").write(tile)
    open("thumb.jpg", "wb").write(fetch(f"{DATA}/{dataset['id']}/thumb.jpg"))
    print(f"{width} × {height}: tile ({col}, {row}) of level {level} of {top}, {len(tile)} bytes, saved as"
          f" tile.jpeg; thumb.jpg too")
    print(f"from the snap to the first tile: {shown - started:.2f} s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
