#!/usr/bin/env python3
"""The app's side of the server contract, end to end, standard library only.

  accounts_e2e.py open   --base URL --realmd HOST:PORT --realm NAME --world ADDRESS:PORT
  accounts_e2e.py token  --base URL --token TOKEN
  accounts_e2e.py closed --base URL

open    sign-up on (INILLA_SIGNUP=lan, no token): GET /server.json describes the realm and no game
        data; POST /accounts creates an account (201), a replay is 200, the name in any case is taken
        (409), invalid bodies are 400/413/415 without reaching the database, and the sixth attempt
        that does reach it from one address is 429. Then the account logs in to realmd with SRP6 and
        the realm list holds the realm; a wrong password is refused.
token   INILLA_DATA_TOKEN set: server.json says so and stays open; sign-up needs the token (401).
closed  INILLA_SIGNUP=off: server.json offers no sign-up, and POST /accounts is 403.

Each phase waits up to --wait seconds for the data host to answer. The salt and verifier are made
here, as the app makes them on the phone: the password never goes over HTTP.
"""
import argparse
import json
import secrets
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import srp6

PASSED = []


def check(cond, what, detail=""):
    if not cond:
        raise SystemExit(f"FAIL: {what}" + (f"\n      {detail}" if detail else ""))
    PASSED.append(what)
    print(f"  ok  {what}")


def request(url, body=None, content_type="application/json", token=None, method=None):
    """(status, headers, parsed JSON or None, raw text) for one request; HTTP errors are answers too."""
    headers = {}
    data = None
    if body is not None:
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        headers["Content-Type"] = content_type
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            status, hdrs, raw = r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        status, hdrs, raw = e.code, e.headers, e.read()
    text = raw.decode("utf-8", "replace")
    try:
        doc = json.loads(text)
    except ValueError:
        doc = None
    return status, hdrs, doc, text


def wait_for(url, seconds):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            status, _, _, _ = request(url)
            if status == 200:
                return
            last = f"HTTP {status}"
        except OSError as e:
            last = str(e)
        time.sleep(2)
    raise SystemExit(f"FAIL: {url} did not answer 200 within {seconds} s (last: {last})")


def server_json(base):
    status, hdrs, doc, text = request(urllib.parse.urljoin(base, "server.json"))
    check(status == 200, "GET /server.json is 200", f"{status}: {text[:300]}")
    check(isinstance(doc, dict), "server.json is a JSON object", text[:300])
    check("no-store" in (hdrs.get("Cache-Control") or ""), "server.json is Cache-Control: no-store",
          str(hdrs.get("Cache-Control")))
    check(doc.get("format") == 1, "server.json format 1", str(doc.get("format")))
    return doc


def account_body(name, password, salt=None):
    _, s, v = srp6.make_row(name, password, salt)
    return {"username": name, "salt": s, "verifier": v}


def reply(status, doc, text, want_status, want_ok, error=None, what=""):
    check(status == want_status, f"{what}: HTTP {want_status}", f"got {status}: {text[:300]}")
    check(isinstance(doc, dict) and doc.get("ok") is want_ok and isinstance(doc.get("message"), str),
          f"{what}: JSON reply with ok={str(want_ok).lower()} and a message", text[:300])
    if error:
        check(doc.get("error") == error, f"{what}: error {error}", text[:300])


def phase_open(a):
    doc = server_json(a.base)
    check(doc.get("build") == 5875, "server.json build 5875", str(doc.get("build")))
    check(doc.get("core") == "vmangos", "server.json core vmangos", str(doc.get("core")))
    check(doc.get("name") == a.realm, f"server.json name {a.realm!r}", str(doc.get("name")))
    check(isinstance(doc.get("software"), str) and doc["software"], "server.json names its software",
          str(doc.get("software")))
    realm_host, realm_port = a.realmd.rsplit(":", 1)
    auth = doc.get("auth") or {}
    check(auth.get("port") == int(realm_port), f"server.json auth.port {realm_port}", json.dumps(auth))
    check("host" not in auth, "server.json names no auth host (the typed host)", json.dumps(auth))
    realms = doc.get("realms") or []
    world_address, world_port = a.world.rsplit(":", 1)
    check(len(realms) == 1 and realms[0].get("name") == a.realm and realms[0].get("kind") == "pve",
          f"server.json lists the one realm {a.realm!r} (pve)", json.dumps(realms))
    world = realms[0].get("world") or {}
    check(world.get("address") == world_address and world.get("port") == int(world_port)
          and world.get("same_host") is True,
          f"server.json realm world {a.world}, same_host", json.dumps(world))
    check(doc.get("data") is None, "server.json offers no game data (none imported)", json.dumps(doc.get("data")))
    reg = doc.get("registration") or {}
    check(reg.get("open") is True and reg.get("token") is False and reg.get("credentials") == "srp6",
          "server.json registration open, no token, srp6", json.dumps(reg))
    check((reg.get("name_min"), reg.get("name_max"), reg.get("password_min"), reg.get("password_max"))
          == (3, 16, 6, 16), "server.json registration limits 3-16 / 6-16", json.dumps(reg))
    url = urllib.parse.urljoin(a.base, reg.get("url", ""))
    check(url == urllib.parse.urljoin(a.base, "accounts"), "registration.url resolves to /accounts", url)

    name = "ci" + secrets.token_hex(5)  # lower case on purpose: stored and answered upper-case
    password = "Ci-" + secrets.token_hex(6)
    body = account_body(name, password)
    db_attempts = 0

    status, _, doc, text = request(url, body)
    db_attempts += 1
    reply(status, doc, text, 201, True, what="POST /accounts, a new name")
    check(doc.get("username") == name.upper(), "the reply names the account upper-case", text)

    status, _, doc, text = request(url, body)
    db_attempts += 1
    reply(status, doc, text, 200, True, what="the same request again (a replay)")
    check(doc.get("username") == name.upper(), "the replay names the account", text)

    for taken in (name.upper(), name):
        status, _, doc, text = request(url, account_body(taken, "Other-" + secrets.token_hex(4)))
        db_attempts += 1
        reply(status, doc, text, 409, False, "name_taken", what=f"{taken} with another salt")

    # Refused before the database: none of these count towards the rate limit.
    salt_ok, ver_ok = body["salt"], body["verifier"]
    invalid = [
        ({"username": "ab", "salt": salt_ok, "verifier": ver_ok}, "invalid_name", "a 2-letter name"),
        ({"username": "a" * 17, "salt": salt_ok, "verifier": ver_ok}, "invalid_name", "a 17-letter name"),
        ({"username": "bad_name", "salt": salt_ok, "verifier": ver_ok}, "invalid_name", "a name with '_'"),
        ({"username": "ci" + secrets.token_hex(4), "salt": "7" + salt_ok[1:], "verifier": ver_ok},
         "invalid_credentials", "a salt without its top bit"),
        ({"username": "ci" + secrets.token_hex(4), "salt": salt_ok[:62], "verifier": ver_ok},
         "invalid_credentials", "a 62-digit salt"),
        ({"username": "ci" + secrets.token_hex(4), "salt": salt_ok[:-1] + "G", "verifier": ver_ok},
         "invalid_credentials", "a salt that is not hex"),
        ({"username": "ci" + secrets.token_hex(4), "salt": salt_ok, "verifier": "0"},
         "invalid_credentials", "a zero verifier"),
        ({"username": "ci" + secrets.token_hex(4), "salt": salt_ok, "verifier": f"{srp6.N:X}"},
         "invalid_credentials", "a verifier equal to N"),
        ({"username": "ci" + secrets.token_hex(4), "salt": salt_ok, "verifier": "1" * 65},
         "invalid_credentials", "a 65-digit verifier"),
    ]
    for payload, error, what in invalid:
        status, _, doc, text = request(url, payload)
        reply(status, doc, text, 400, False, error, what=what)
    status, _, doc, text = request(url, b'{"username": "ci', content_type="application/json")
    reply(status, doc, text, 400, False, "bad_request", what="a body that is not JSON")
    status, _, doc, text = request(url, json.dumps(body).encode(), content_type="text/plain")
    reply(status, doc, text, 415, False, "unsupported_media_type", what="a text/plain body")
    status, _, doc, text = request(url, json.dumps(dict(body, pad="x" * 5000)).encode())
    reply(status, doc, text, 413, False, "too_large", what="a body over 4 KiB")

    err, realms = srp6.logon(realm_host, int(realm_port), name, password)
    check(err is None, f"SRP6 logon to {a.realmd} as {name.upper()}", str(err))
    check(any(r[0] == a.realm for r in realms), f"the realm list holds {a.realm!r}", str(realms))
    err, _ = srp6.logon(realm_host, int(realm_port), name, password + "x")
    check(err is not None and "0x04" in err, "a wrong password is refused (0x04)", str(err))

    # Per address, 5 attempts that reach the database in 10 minutes; the sixth is 429.
    while db_attempts < 5:
        status, _, doc, text = request(url, account_body("ci" + secrets.token_hex(5), password))
        db_attempts += 1
        reply(status, doc, text, 201, True, what=f"database attempt {db_attempts} of 5")
    status, hdrs, doc, text = request(url, account_body("ci" + secrets.token_hex(5), password))
    reply(status, doc, text, 429, False, "rate_limited", what="the sixth database attempt")
    retry = hdrs.get("Retry-After") or ""
    check(retry.isdigit() and int(retry) > 0 and doc.get("retry_after") == int(retry),
          "429 says when to retry (Retry-After and retry_after)", f"{retry!r}: {text[:300]}")


def phase_token(a):
    doc = server_json(a.base)
    reg = doc.get("registration") or {}
    check(reg.get("open") is True and reg.get("token") is True, "server.json: sign-up needs the token",
          json.dumps(reg))
    url = urllib.parse.urljoin(a.base, reg.get("url", "accounts"))
    name, password = "ci" + secrets.token_hex(5), "Ci-" + secrets.token_hex(6)
    body = account_body(name, password)
    status, _, doc, text = request(url, body)
    reply(status, doc, text, 401, False, "token_required", what="POST /accounts without the token")
    status, _, doc, text = request(url, body, token=a.token + "x")
    reply(status, doc, text, 401, False, "token_required", what="POST /accounts with a wrong token")
    status, _, doc, text = request(url, body, token=a.token)
    reply(status, doc, text, 201, True, what="POST /accounts with the token")


def phase_closed(a):
    doc = server_json(a.base)
    reg = doc.get("registration")
    check(reg is None or reg.get("open") is False, "server.json: no sign-up", json.dumps(reg))
    url = urllib.parse.urljoin(a.base, "accounts")
    status, _, doc, text = request(url, account_body("ci" + secrets.token_hex(5), "Ci-" + secrets.token_hex(6)))
    reply(status, doc, text, 403, False, "registration_closed", what="POST /accounts with sign-up off")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("phase", choices=["open", "token", "closed"])
    p.add_argument("--base", required=True, help="the data host, e.g. http://127.0.0.1:8099/")
    p.add_argument("--realmd", default="127.0.0.1:3724")
    p.add_argument("--realm", default="Vanillish")
    p.add_argument("--world", default="127.0.0.1:8085", help="REALM_ADDRESS:MANGOSD_PORT")
    p.add_argument("--token", default="")
    p.add_argument("--wait", type=int, default=120)
    a = p.parse_args()
    if not a.base.endswith("/"):
        a.base += "/"
    check(srp6.golden(), "SRP6 verifier matches benilla-srp's golden value")
    wait_for(urllib.parse.urljoin(a.base, "server.json"), a.wait)
    {"open": phase_open, "token": phase_token, "closed": phase_closed}[a.phase](a)
    print(f"{a.phase}: {len(PASSED)} checks passed")


if __name__ == "__main__":
    main()
