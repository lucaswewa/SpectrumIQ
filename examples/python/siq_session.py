"""Logs an example script in to SpectrumIQ (ADR-0065), and takes control of the instrument if it
changes it (ADR-0066): every urllib request after then carries the session's cookie, the token is
kept by a heartbeat while the script runs, and released when it ends.

    import siq_session
    siq_session.login(ROOT, control=True)

The user and the password are SIQ_USER's and SIQ_PASSWORD's. Without SIQ_USER, nothing is done:
the service may not ask for a login. Add a user first, with `spectrumiq-server user add`. It needs
only the standard library.
"""

import atexit
import http.cookiejar
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.request

# A third of the service's 30 s: the token stays while the script is quiet, waiting.
HEARTBEAT_S = 10


def _post(url, body=None):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method="POST", headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request) as answer:
        return json.loads(answer.read() or b"null")


def login(root, control=False):
    """Logs in as SIQ_USER, and if `control`, takes the control token until the script ends."""
    user = os.environ.get("SIQ_USER")
    if not user:
        return
    jar = http.cookiejar.CookieJar()
    urllib.request.install_opener(urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar)))
    try:
        me = _post(root + "/api/siq/auth/login", {"name": user, "password": os.environ.get("SIQ_PASSWORD", "")})
        print(f"logged in as {me['name']}, {me['role']}")
        if control:
            _post(root + "/api/siq/control/request")
            print("took control of the instrument")
    except urllib.error.HTTPError as refused:
        detail = json.loads(refused.read() or b"{}").get("detail", refused.reason)
        sys.exit(f"{root} refused: {detail}")
    if control:

        def beat():
            while True:
                time.sleep(HEARTBEAT_S)
                try:
                    _post(root + "/api/siq/control/heartbeat")
                except OSError:
                    return

        threading.Thread(target=beat, daemon=True).start()
        atexit.register(lambda: _post(root + "/api/siq/control/release"))
