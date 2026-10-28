"""Takes a z-stack where the stage is, through the `workflow` Thing.

Start the service first, with the simulated instrument:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml
    set SIQ_USER=<a user>& set SIQ_PASSWORD=<its password>   (siq_session.py)

then, from the repository:

    python examples/python/z_stack.py [http://127.0.0.1:5000]

It takes 11 planes 2 um apart about where the focus is, a frame in brightfield and in DAPI at
each, going up, and prints each frame as the run reports it, `z` numbering the planes from the
bottom; then the focus is back in the middle. It needs only the standard library.
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


def main():
    middle = call("GET", f"{API}/focus/position")
    # At each plane, each channel and a frame: the plan's own steps.
    steps = [{"step": "channel", "id": 1}, {"step": "snap"}, {"step": "channel", "id": 2}, {"step": "snap"}]
    stack = {"name": "z-stack", "planes": {"range_um": 20.0, "step_um": 2.0, "direction": "up"}, "steps": steps}
    invocation = call("POST", f"{API}/workflow/z_stack", stack)
    href, seen = invocation["href"], None
    while invocation["status"] in ("pending", "running"):
        time.sleep(0.2)
        now = call("GET", f"{API}/workflow/progress")
        if now and now["step"] != seen and now["step"].startswith("snap"):
            seen = now["step"]
            print(f"  {seen}")
        invocation = call("GET", href)
    if invocation["status"] != "completed":
        raise RuntimeError(f"workflow.z_stack: {invocation}")
    finished = invocation["output"]
    back = call("GET", f"{API}/focus/position")
    print(f"{finished['frames']} frames in dataset {finished['dataset']}; the focus back at {back:.1f} um"
          f" (from {middle:.1f})")
    return 0


if __name__ == "__main__":
    siq_session.login(ROOT, control=True)
    sys.exit(main())
