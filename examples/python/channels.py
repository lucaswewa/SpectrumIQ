"""Cycles through the configured channels and saves a snap of each, through the HTTP API.

Start the service first, with the simulated instrument:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml

then, from the repository:

    python examples/python/channels.py [http://127.0.0.1:5000]

For each channel, it selects the channel (every light off, the filter, the exposure and the
display colour, then the channel's light), shows the light path, and saves `channel-<id>.jpg` in
the channel's colour. Then it shows the pool and the process's memory. It needs only the standard
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


def main():
    sources = call("GET", f"{API}/illumination/sources")
    filters = call("GET", f"{API}/filter_wheel/filters")
    for channel in call("GET", f"{API}/imaging/channels"):
        invoke("imaging", "select_channel", {"id": channel["id"]})
        levels = call("GET", f"{API}/illumination/levels")
        lit = [f"{name} at {level:.0%}" for name, level in zip(sources, levels) if level > 0]
        position = call("GET", f"{API}/filter_wheel/position")
        print(f"{channel['id']} {channel['name']}: {', '.join(lit) or 'dark'}, through {filters[position]},"
              f" {channel['exposure']} ms")
        snap = invoke("camera", "snap", {})
        with urllib.request.urlopen(snap["href"]) as jpeg:
            open(f"channel-{channel['id']}.jpg", "wb").write(jpeg.read())
    memory = call("GET", f"{ROOT}/api/siq/system/memory")
    pool, process = memory["pool"], memory["process"] or {}
    print(f"pool: {pool['slots']} slots of {pool['slot_bytes'] / 1e6:.1f} MB, {pool['free']} free;"
          f" the process: {process.get('working_set', 0) / 1e6:.0f} MB resident")
    return 0


if __name__ == "__main__":
    sys.exit(main())
