"""Stitches a tile scan's dataset through the `jobs` Thing, follows the job, and reads its QA.

Start the service first, with the simulated instrument:

    cargo run --release -p spectrumiq-server -- -c config/sim.toml
    set SIQ_USER=<a user>& set SIQ_PASSWORD=<its password>   (siq_session.py)

then, from the repository:

    python examples/python/stitch_dataset.py [http://127.0.0.1:5000] [dataset id]

Without an id, it stitches the newest tile scan; run examples/python/tile_scan.py first to make
one. The mosaic is the dataset's image `stitched-again`, beside the `stitched` that the service
made by itself when the scan finished. It prints the job's progress, the stitch's measures from the
dataset's history, and the pairs its QA report found worst. It needs only the standard library.
"""

import json
import sys
import time
import urllib.request

import siq_session

ROOT = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5000").rstrip("/")
API = ROOT + "/api/v1"
DATA = ROOT + "/api/siq/data"
IMAGE = "stitched-again"


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
        time.sleep(0.25)
        invocation = call("GET", invocation["href"])
    if invocation["status"] != "completed":
        raise SystemExit(f"{thing}.{action} failed: {invocation.get('error')}")
    return invocation.get("output")


def newest_scan():
    """The newest tile scan's id."""
    page = invoke("datasets", "summary", {"offset": 0, "limit": 100})
    scans = [entry for entry in page["datasets"] if entry["kind"] == "tile_scan"]
    if not scans:
        raise SystemExit("there is no tile scan: run examples/python/tile_scan.py first")
    return scans[0]["id"]


def main():
    dataset = sys.argv[2] if len(sys.argv) > 2 else newest_scan()
    job = invoke("jobs", "stitch", {"dataset": dataset, "image": IMAGE})
    print(f"stitching {dataset} into {IMAGE}: job {job}")
    # Followed while it waits or runs.
    while True:
        jobs = call("GET", f"{API}/jobs/running") + call("GET", f"{API}/jobs/queue")
        mine = next((each for each in jobs if each["id"] == job), None)
        if mine is None:
            break
        print(f"  {mine['state']}: {mine['progress'] * 100:.0f} %  {mine.get('message') or ''}")
        time.sleep(1)

    # How it went, as the dataset's history records it.
    history = call("GET", f"{DATA}/{dataset}/manifest")["history"]
    record = [entry for entry in history if entry["kind"] == "stitch"][-1]
    if record["state"] != "done":
        raise SystemExit(f"the stitch {record['state']}: {record.get('error')}")
    results = record["results"]
    print(
        f"{results['tiles']} tiles, {results['pairs']} pairs, {results['trusted']} trusted, "
        f"{results['rejected']} rejected; the edges used at {results['rms_px']:.3f} px RMS, "
        f"{results['max_px']:.3f} px at worst; {results['components']} component(s)"
    )
    info = call("GET", f"{DATA}/{dataset}/{IMAGE}/info")
    print(f"the mosaic: {info['size']['x']} x {info['size']['y']} pixels, {info['levels']} levels")

    # The pairs its QA found worst, from the overlays the viewer draws.
    residuals = call("GET", f"{DATA}/{dataset}/{IMAGE}/overlays")[1]["shapes"]
    worst = [line for line in residuals if line["tone"] in ("danger", "warning", "muted")]
    for line in worst[:5]:
        print(f"  {line['title']}")
    print(f"view it at {ROOT}/#/viewer?dataset={dataset}&image={IMAGE}")


if __name__ == "__main__":
    siq_session.login(ROOT, control=False)
    main()
