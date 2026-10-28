"""Two clients and the Hardware Control Token (ADR-0066): a viewer is refused a change; an operator
is refused it until she holds the token, then makes it; a second operator is refused the token
while she holds it, but may stop the stage; and once she releases it, he holds it.

The token is enforced only where `[auth] enforce` is true, and config/sim.toml leaves it off
until the web app logs in. From the repository, with a copy of it, users, and the service:

    copy config\\sim.toml config\\enforced.toml        then set `enforce = true` in its [auth]
    spectrumiq-server user add ada --role admin -c config/enforced.toml
    spectrumiq-server user add olga --role operator -c config/enforced.toml
    spectrumiq-server user add otto --role operator -c config/enforced.toml
    spectrumiq-server user add vic --role viewer -c config/enforced.toml
    cargo run --release -p spectrumiq-server -- -c config/enforced.toml

then:

    python examples/python/two_clients.py [http://127.0.0.1:5000] [password]

Each user's password is the one given, `correct horse` by default. Each client keeps its own
session cookie. It needs only the standard library, and fails if the service answers otherwise.
"""

import http.cookiejar
import json
import sys
import urllib.error
import urllib.request

ROOT = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:5000").rstrip("/")
PASSWORD = sys.argv[2] if len(sys.argv) > 2 else "correct horse"
API = ROOT + "/api/v1"
CONTROL = ROOT + "/api/siq/control"


class Client:
    """A browser of its own: a user's session, in its cookie jar."""

    def __init__(self, name):
        self.name = name
        jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
        status, me = self.call("POST", ROOT + "/api/siq/auth/login", {"name": name, "password": PASSWORD})
        if status != 200:
            raise SystemExit(f"{name} couldn't log in ({status}): {me}. Add the users first.")
        print(f"{name} logged in as {me['role']}")

    def call(self, method, url, body=None):
        """Sends a request: its status, and its decoded answer, a refusal's included."""
        data = None if body is None else json.dumps(body).encode()
        headers = {"Content-Type": "application/json"}
        request = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with self.opener.open(request) as answer:
                return answer.status, json.loads(answer.read() or b"null")
        except urllib.error.HTTPError as refused:
            return refused.code, json.loads(refused.read() or b"null")

    def expect(self, what, method, url, body, statuses):
        """Sends a request, and stops the script unless its status is one of `statuses`."""
        status, answer = self.call(method, url, body)
        detail = answer.get("detail") if isinstance(answer, dict) else None
        print(f"  {self.name} {what}: {status}" + (f" ({detail})" if detail else ""))
        if status not in statuses:
            raise SystemExit(f"expected {statuses}, got {status}: {answer}")
        return answer


def exposure(client, ms, statuses):
    return client.expect(f"sets the exposure to {ms} ms", "PUT", f"{API}/camera/exposure_ms", ms, statuses)


def main():
    vic, olga, otto = Client("vic"), Client("olga"), Client("otto")
    held = vic.call("GET", CONTROL)[1]
    if held is None:
        raise SystemExit("the service has no control token: is it SpectrumIQ's?")
    if held["holder"] is not None:
        raise SystemExit(f"{held['holder']['user']} holds the token: release it first")

    print("a viewer, then an operator without the token:")
    exposure(vic, 20.0, [403])
    exposure(olga, 20.0, [403])

    print("olga takes the token:")
    olga.expect("requests the token", "POST", CONTROL + "/request", None, [200])
    exposure(olga, 20.0, [200, 201, 204])
    otto.expect("requests the token", "POST", CONTROL + "/request", None, [409])
    exposure(otto, 15.0, [403])
    otto.expect("stops the stage", "POST", f"{API}/stage/halt", None, [200, 201])

    print("olga hands it over:")
    olga.expect("releases the token", "POST", CONTROL + "/release", None, [204])
    otto.expect("requests the token", "POST", CONTROL + "/request", None, [200])
    exposure(otto, 15.0, [200, 201, 204])
    holder = vic.call("GET", CONTROL)[1]["holder"]
    print(f"the token is {holder['user']}'s since {holder['since']}")
    otto.expect("releases the token", "POST", CONTROL + "/release", None, [204])


if __name__ == "__main__":
    main()
