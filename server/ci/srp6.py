#!/usr/bin/env python3
"""vmangos accounts and a 1.12.1 (build 5875) SRP6 logon client, standard library only.

  srp6.py row USER PASS               prints "USERNAME SALT_HEX VERIFIER_HEX", as vmangos stores them
  srp6.py logon HOST PORT USER PASS   logs in to realmd and prints the realm list; exit 0 on success
  srp6.py golden                      checks the verifier against benilla-srp's golden value

The salt and verifier are what the app posts to POST /accounts: computed where the password is,
written as big-endian upper-case hex (OpenSSL's BN_bn2hex, how vmangos stores `s` and `v`).
The logon is the client's: challenge, proof with the 5875 Win/x86 integrity hash, realm list.
"""
import hashlib
import secrets
import socket
import struct
import sys

N = int("894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7", 16)
G = 7
# allowed_clients 1.12.1 build 5875 Win x86 (vmangos realmd.allowed_clients, StrictVersionCheck = 1).
INTEGRITY_5875_WIN = bytes.fromhex("95EDB27C7823B363CBDDAB56A392E7CB73FCCA20")

# realmd's logon results that matter here (AuthCodes.h).
RESULTS = {0x04: "unknown account or wrong password", 0x08: "throttled (too many failures)",
           0x09: "client version refused", 0x0D: "broken v/s in the database"}


def sha1(*parts):
    h = hashlib.sha1()
    for p in parts:
        h.update(p)
    return h.digest()


def le(n, width=32):
    return n.to_bytes(width, "little")


def norm(s):
    """1.12 / vmangos normalisation: 1..16 ASCII, upper-cased (AccountMgr::normalizeString)."""
    if not (1 <= len(s) <= 16) or not all(0x20 <= ord(c) < 0x7F for c in s):
        raise ValueError("1..16 printable ASCII characters")
    return s.upper()


def verifier(user, password, salt):
    """v = g^x mod N, x = SHA1(salt_le32 | SHA1(USER:PASS)) read little endian (SRP6::CalculateVerifier)."""
    x = int.from_bytes(sha1(le(salt), sha1(f"{norm(user)}:{norm(password)}".encode())), "little")
    return pow(G, x, N)


def bn2hex(n):
    """OpenSSL BN_bn2hex: upper-case, whole bytes, no leading zero bytes."""
    width = max(1, (n.bit_length() + 7) // 8)
    return f"{n:0{width * 2}X}"


def new_salt():
    """BigNumber::SetRand(256) = BN_rand(bn, 256, top=0, bottom=1): 256 bits, top bit set, odd."""
    return secrets.randbits(256) | (1 << 255) | 1


def make_row(user, password, salt=None):
    salt = new_salt() if salt is None else salt
    return norm(user), bn2hex(salt), bn2hex(verifier(user, password, salt))


def recv_exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError(f"realmd closed the connection ({len(buf)}/{n} bytes)")
        buf += chunk
    return buf


def challenge(host, port, user):
    sock = socket.create_connection((host, port), timeout=10)
    name = norm(user).encode()
    body = (b"WoW\0" + bytes([1, 12, 1]) + struct.pack("<H", 5875) + b"68x\0" + b"niW\0" + b"SUne"
            + struct.pack("<I", 0) + bytes([127, 0, 0, 1]) + bytes([len(name)]) + name)
    sock.sendall(bytes([0x00, 3]) + struct.pack("<H", len(body)) + body)
    op, _proto, result = recv_exact(sock, 3)
    if op != 0x00:
        raise RuntimeError(f"unexpected opcode {op:#x}")
    if result != 0:
        sock.close()
        return None, result, None
    B = recv_exact(sock, 32)
    glen = recv_exact(sock, 1)[0]
    g = recv_exact(sock, glen)[0]
    nlen = recv_exact(sock, 1)[0]
    n = recv_exact(sock, nlen)
    salt = recv_exact(sock, 32)
    _crc_salt = recv_exact(sock, 16)
    flags = recv_exact(sock, 1)[0]
    if flags & 1:
        recv_exact(sock, 4 + 16)
    return sock, 0, (int.from_bytes(B, "little"), g, int.from_bytes(n, "little"), int.from_bytes(salt, "little"))


def interleave(S):
    t = le(S)
    e, f = sha1(t[0::2]), sha1(t[1::2])
    return bytes(b for pair in zip(e, f) for b in pair)


def parse_realms(payload):
    """The 1.12 realm list body: u32, u8 count, then per realm u32 type, u8 flags, name, address,
    f32 population, u8 characters, u8 timezone, u8 id."""
    count = payload[4]
    realms, i = [], 5
    for _ in range(count):
        i += 5
        end = payload.index(b"\0", i)
        name = payload[i:end].decode()
        a_end = payload.index(b"\0", end + 1)
        address = payload[end + 1:a_end].decode()
        realms.append((name, address))
        i = a_end + 1 + 4 + 3
    return realms


def logon(host, port, user, password):
    """Logs in; returns (None, realms) on success, (reason, []) when realmd refuses."""
    U, P = norm(user), norm(password)
    # vmangos hashes A, B, K, S at their minimal width; the client at full width. Use only draws where
    # both agree (high byte of A/B/K non-zero, low byte of S non-zero), as benilla-srp does.
    for _dial in range(20):
        sock, result, ch = challenge(host, port, U)
        if result != 0:
            return f"challenge refused: result {result:#04x} ({RESULTS.get(result, '?')})", []
        B, g, n, salt = ch
        if le(B)[31] != 0:
            break
        sock.close()
    else:
        return "realmd kept answering with a short B", []
    x = int.from_bytes(sha1(le(salt), sha1(f"{U}:{P}".encode())), "little")
    for _draw in range(1000):
        a = secrets.randbits(256)
        A = pow(g, a, n)
        if le(A)[31] == 0:
            continue
        u = int.from_bytes(sha1(le(A), le(B)), "little")
        S = pow((B - 3 * pow(g, x, n)) % n, a + u * x, n)
        if le(S)[0] == 0:
            continue
        K = interleave(S)
        if K[39] == 0:
            continue
        hn, hg = sha1(le(n)), sha1(bytes([g]))
        M1 = sha1(bytes(p ^ q for p, q in zip(hn, hg)), sha1(U.encode()), le(salt), le(A), le(B), K)
        break
    else:
        return "no usable SRP6 draw", []
    crc = sha1(le(A), INTEGRITY_5875_WIN)
    sock.sendall(bytes([0x01]) + le(A) + M1 + crc + bytes([0, 0]))
    op, result = recv_exact(sock, 2)
    if op != 0x01 or result != 0:
        sock.close()
        return f"proof refused: opcode {op:#x} result {result:#04x} ({RESULTS.get(result, '?')})", []
    M2 = recv_exact(sock, 20)
    recv_exact(sock, 4)
    if M2 != sha1(le(A), M1, K):
        sock.close()
        return "server proof M2 mismatch", []
    sock.sendall(bytes([0x10]) + struct.pack("<I", 0))
    op = recv_exact(sock, 1)[0]
    size = struct.unpack("<H", recv_exact(sock, 2))[0]
    payload = recv_exact(sock, size)
    sock.close()
    if op != 0x10:
        return f"realm list: unexpected opcode {op:#x}", []
    return None, parse_realms(payload)


def golden():
    """benilla-srp's password_verifier_golden: salt byte i = i*11+2 (little endian), alice/password1."""
    salt = int.from_bytes(bytes((i * 11 + 2) & 0xFF for i in range(32)), "little")
    return le(verifier("alice", "password1", salt)).hex() == \
        "28b837075a12b82553921d9095fa3fdcb0151c4bfc860ab97a69d0fa86a3d213"


def main(argv):
    if argv[:1] == ["row"] and len(argv) == 3:
        print(*make_row(argv[1], argv[2]))
    elif argv[:1] == ["logon"] and len(argv) == 5:
        err, realms = logon(argv[1], int(argv[2]), argv[3], argv[4])
        if err:
            print(err)
            return 1
        print(f"logon OK as {norm(argv[3])}; realms: " + ", ".join(f"{n} ({a})" for n, a in realms))
    elif argv[:1] == ["golden"]:
        ok = golden()
        print("golden", "MATCH" if ok else "MISMATCH")
        return 0 if ok else 1
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
