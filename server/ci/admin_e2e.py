#!/usr/bin/env python3
"""The app's side of the admin menu's door at the data host, end to end, standard library only.

  ADMIN_CODE=... admin_e2e.py on --base URL
  admin_e2e.py off --base URL
  admin_e2e.py vectors

on       ADMIN_CODE set, INILLA_ADMIN=lan, asked from a private address, no mangosd: server.json has
         the admin section (open); GET /admin/nonce gives a fresh 32-hex nonce; GET /admin/status
         without the header or with a malformed one is 401 admin_auth, with a wrong MAC 401
         admin_code, with the right one 503 world_unavailable (the agent lives in mangosd, which is
         off), and its nonce again 401 admin_nonce; POST /admin/rates signed right is 503 too; a
         wrong method is 405 with Allow, an unknown route 404 not_found.
off      INILLA_ADMIN=off: server.json has no admin section, and /admin/nonce and /admin/status are
         404 admin_off.
vectors  the MAC of the two vectors the data host's and the app's tests share, and the rule for a
         usable code (the setup's 16 letters, or 20 or more hex digits; never change-me).

The code comes from the environment, never an argument, and is never printed. Each request is
signed as the app signs it: HMAC-SHA256(SHA-256(the normalised code), nonce \\n METHOD \\n route \\n
hex SHA-256(body)), in `Authorization: Vanillish-Admin <nonce>:<mac>`. Each phase waits up to --wait
seconds for the data host to answer.
"""
import argparse
import hashlib
import hmac
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

PASSED = []

# (code, nonce, method, route, body) -> the header, as the data host's and inilla-core's tests assert.
VECTORS = [
    (("ABCD-EFGH-JKLM-NPQR", "00112233445566778899aabbccddeeff", "GET", "/admin/status", b""),
     "Vanillish-Admin 00112233445566778899aabbccddeeff:"
     "2bfb9cca1911d025fc88f5222621f3ae5f2e9635d4c363fdc0231980353acbf4"),
    (("ABCD-EFGH-JKLM-NPQR", "00112233445566778899aabbccddeeff", "POST", "/admin/rates",
      b'{"rates":{"Rate.XP.Kill":2},"announce":true}'),
     "Vanillish-Admin 00112233445566778899aabbccddeeff:"
     "af86f2b02789533c33b4c3a704701551d9517aa95581ecb26739906cb3efe176"),
]


def check(cond, what, detail=""):
    if not cond:
        raise SystemExit(f"FAIL: {what}" + (f"\n      {detail}" if detail else ""))
    PASSED.append(what)
    print(f"  ok  {what}")


def normalize(code):
    """ASCII whitespace as Rust's is_ascii_whitespace reads it (space, tab, LF, FF, CR: no vertical
    tab) and '-' dropped, ASCII upper case; anything else not a letter or digit: None."""
    out = []
    for ch in code:
        if ch in " \t\n\r\f-":
            continue
        if not (ch.isascii() and ch.isalnum()):
            return None
        out.append(ch.upper())
    return "".join(out)


SETUP_LETTERS = set("ABCDEFGHJKLMNPQRSTUVWXYZ23456789")


def usable(code):
    """The data host's, the agent's and the app's rule: a random code, normalised, not change-me:
    16 or more of the setup's letters, or 20 or more hex digits (openssl rand -hex 10)."""
    n = normalize(code)
    if n is None or n.startswith("CHANGEME"):
        return False
    return (len(n) >= 16 and set(n) <= SETUP_LETTERS) or (len(n) >= 20 and set(n) <= set("0123456789ABCDEF"))


def auth_header(code, nonce, method, route, body):
    key = hashlib.sha256(normalize(code).encode("ascii")).digest()
    msg = f"{nonce}\n{method}\n{route}\n{hashlib.sha256(body).hexdigest()}"
    return f"Vanillish-Admin {nonce}:{hmac.new(key, msg.encode('ascii'), hashlib.sha256).hexdigest()}"


def request(url, method="GET", body=None, auth=None):
    """(status, headers, parsed JSON or None, raw text) for one request; HTTP errors are answers too."""
    headers = {}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if auth:
        headers["Authorization"] = auth
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=40) as r:
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
    status, _, doc, text = request(urllib.parse.urljoin(base, "server.json"))
    check(status == 200 and isinstance(doc, dict), "GET /server.json is a 200 JSON object", f"{status}: {text[:300]}")
    return doc


def refused(answer, want_status, error, what):
    status, _, doc, text = answer
    check(status == want_status, f"{what}: HTTP {want_status}", f"got {status}: {text[:300]}")
    check(isinstance(doc, dict) and doc.get("ok") is False and isinstance(doc.get("message"), str),
          f"{what}: JSON reply with ok=false and a message", text[:300])
    if error:
        check(doc.get("error") == error, f"{what}: error {error}", text[:300])


def nonce(admin):
    status, _, doc, text = request(urllib.parse.urljoin(admin, "nonce"))
    check(status == 200 and isinstance(doc, dict) and doc.get("ok") is True, "GET /admin/nonce is 200, ok",
          f"{status}: {text[:300]}")
    n = doc.get("nonce")
    check(isinstance(n, str) and re.fullmatch(r"[0-9a-f]{32}", n) is not None, "the nonce is 32 lowercase hex",
          text[:300])
    check(doc.get("expires_in") == 60, "the nonce lasts 60 s", text[:300])
    return n


def phase_on(a):
    code = os.environ.get("ADMIN_CODE", "")
    if not usable(code):
        raise SystemExit("FAIL: set ADMIN_CODE to the data host's code (the setup's 16 letters, or 20 or more hex "
                         "digits)")
    doc = server_json(a.base)
    section = doc.get("admin")
    check(isinstance(section, dict) and section.get("url") == "admin/" and section.get("open") is True,
          "server.json: admin {url: admin/, open: true} (asked from a private address)", json.dumps(section))
    admin = urllib.parse.urljoin(a.base, section["url"])
    status_url, rates_url = urllib.parse.urljoin(admin, "status"), urllib.parse.urljoin(admin, "rates")

    first, second = nonce(admin), nonce(admin)
    check(first != second, "each nonce is a new one")
    refused(request(status_url), 401, "admin_auth", "GET /admin/status without Authorization")
    refused(request(status_url, auth=f"Vanillish-Admin {second}"), 401, "admin_auth",
            "GET /admin/status with a malformed Authorization")
    refused(request(status_url, auth=f"Bearer {code}"), 401, "admin_auth",
            "GET /admin/status with the code itself as a bearer token")
    # A wrong code: the right one with its last letter changed, signed the same way.
    wrong = normalize(code)[:-1] + ("B" if normalize(code)[-1] == "A" else "A")
    refused(request(status_url, auth=auth_header(wrong, first, "GET", "/admin/status", b"")), 401, "admin_code",
            "GET /admin/status signed with a wrong code")
    signed = auth_header(code, second, "GET", "/admin/status", b"")
    refused(request(status_url, auth=signed), 503, "world_unavailable",
            "GET /admin/status signed right, no world server")
    refused(request(status_url, auth=signed), 401, "admin_nonce", "the same nonce again")
    # Signed for another route: the MAC covers the route.
    refused(request(status_url, auth=auth_header(code, nonce(admin), "GET", "/admin/gold", b"")), 401, "admin_code",
            "GET /admin/status signed for /admin/gold")
    body = json.dumps({"rates": {"Rate.XP.Kill": 2}, "announce": False}, separators=(",", ":")).encode()
    refused(request(rates_url, "POST", body, auth_header(code, nonce(admin), "POST", "/admin/rates", body)),
            503, "world_unavailable", "POST /admin/rates signed right, no world server")
    refused(request(rates_url, "POST", body, auth_header(code, nonce(admin), "POST", "/admin/rates", b"{}")),
            401, "admin_code", "POST /admin/rates with a body other than the one signed")
    answer = request(rates_url)
    refused(answer, 405, None, "GET /admin/rates")
    check("POST" in (answer[1].get("Allow") or ""), "405 says Allow: POST", str(answer[1].get("Allow")))
    refused(request(urllib.parse.urljoin(admin, "nope")), 404, "not_found", "GET /admin/nope")


def phase_off(a):
    doc = server_json(a.base)
    check("admin" not in doc, "server.json: no admin section", json.dumps(doc.get("admin")))
    for route in ("nonce", "status"):
        refused(request(urllib.parse.urljoin(a.base, f"admin/{route}")), 404, "admin_off", f"GET /admin/{route}")


def vectors():
    for (args, want) in VECTORS:
        check(auth_header(*args) == want, f"the shared vector {args[2]} {args[3]}", auth_header(*args))
    check(normalize("abcd efgh-jklm\tnpqr") == "ABCDEFGHJKLMNPQR", "normalised: no spaces or dashes, upper case")
    check(normalize("ABCD-EFGH-JKLM-NPQß") is None and normalize("ABCD_EFGH") is None,
          "normalised: anything else is no code")
    check(normalize("ABCD\x0bEFGH") is None and not usable("ABCD\x0bEFGH-JKLM-NPQR"),
          "normalised: a vertical tab is no whitespace (as is_ascii_whitespace reads it)")
    check(all(usable(c) for c in ("ABCD-EFGH-JKLM-NPQR", "abcd efgh jklm npqr", "0123456789abcdef0123",
                                  "0123-4567-89AB-CDEF-0123-45")),
          "usable: the setup's 16 letters, or 20 or more hex digits, in any case, with dashes or spaces")
    check(not any(usable(c) for c in ("", "ABCD-EFGH-JKLM-NPQ", "ABCDEFGHIJKLMNOP", "VanillishServer2024",
                                      "0123456789abcdef012", "CHANGE-ME-ABCD-EFGH-JKLM", "changemeabcdefghjk",
                                      "ABCD-EFGH-JKLM-NPQß")),
          "usable: not a hand-picked, short, change-me or non-ASCII code")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("phase", choices=["on", "off", "vectors"])
    p.add_argument("--base", help="the data host, e.g. http://127.0.0.1:8099/")
    p.add_argument("--wait", type=int, default=120)
    a = p.parse_args()
    vectors()
    if a.phase != "vectors":
        if not a.base:
            p.error("--base is required")
        if not a.base.endswith("/"):
            a.base += "/"
        wait_for(urllib.parse.urljoin(a.base, "server.json"), a.wait)
        {"on": phase_on, "off": phase_off}[a.phase](a)
    print(f"admin {a.phase}: {len(PASSED)} checks passed")


if __name__ == "__main__":
    sys.exit(main())
