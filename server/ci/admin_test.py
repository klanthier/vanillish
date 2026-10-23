#!/usr/bin/env python3
"""The admin agent (docker/vmangos/bin/vanillish-admin) on a plain host, standard library only.

  python3 -I server/ci/admin_test.py [-v] [Test.name ...]

Temporary folders stand in for /run/vmangos, /vmangos/admin, /opt/vmangos/vanillish-patches and
/proc/net (the agent's INILLA_ADMIN_* variables); a thread plays mangosd's console on a real FIFO
(each line read, log noise, the answer and `mangos>` appended to console.log as tee -a does); a stub
`mariadb` first on PATH plays the characters database and the world database's mangos_string. The
API runs in this process, its timings cut short; `render`, `serve` and vmangos-console also run as
the image runs them.
"""
import contextlib
import fcntl
import http.client
import importlib.machinery
import importlib.util
import json
import os
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.normpath(os.path.join(HERE, "..", "docker", "vmangos", "bin"))
AGENT = os.path.join(BIN, "vanillish-admin")
_loader = importlib.machinery.SourceFileLoader("vanillish_admin", AGENT)
admin = importlib.util.module_from_spec(importlib.util.spec_from_loader(_loader.name, _loader))
_loader.exec_module(admin)

CODE = "ABCD-EFGH-JKLM-NPQR"
DB_PASSWORD = "db-secret-42"
LOOT_QUALITIES = ("Poor", "Normal", "Uncommon", "Rare", "Epic", "Legendary", "Referenced")
# A mangosd.conf as inilla-mangosd leaves it, cut down: Rate.XP.Kill twice (the first one counts),
# Rate.XP.Quest in lower case (mangosd does not read it: 1), gold 1.5 from config/mangosd.conf.
BASE = (
    b"ConfVersion=2025040601\n"
    b'LoginDatabase.Info = "db;3306;mangos;' + DB_PASSWORD.encode() + b';realmd"\n'
    b"# Rate.XP.Kill = 9\n"
    b"Rate.XP.Kill = 1\n"
    b"Rate.XP.Kill = 7\n"
    b"  rate.xp.quest=1\n"
    b"Rate.XP.Explore = 1\n"
    + b"".join(b"Rate.Drop.Item.%s = 1\n" % q.encode() for q in LOOT_QUALITIES)
    + b"Rate.Drop.Money = 1.5\n"
    b'Motd = "Welcome to Vanillish \xc3\xa9"\n'
    b"Odd.Line = x\r\n"
    b"mmap.enabled = 0\n"
)
TCP_HEAD = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid\n"
TCP_8091 = "   0: 00000000:1F9B 00000000:0000 0A 00000000:00000000 00:00000000 00000000   999\n"
TCP_8085 = "   1: 00000000:1F95 00000000:0000 0A 00000000:00000000 00:00000000 00000000   999\n"
CHARACTERS = [
    {"name": "Thrall", "level": 12, "race": 2, "class": 1, "online": 1},
    {"name": "Jaina", "level": 20, "race": 1, "class": 8, "online": 1},
    {"name": "Élise", "level": 5, "race": 1, "class": 5, "online": 1},
    {"name": "Bob", "level": 1, "race": 3, "class": 1, "online": 0},
    {"name": "Gone", "level": 60, "race": 5, "class": 4, "online": 0, "deleted": True},
]
GIFT = '"A gift from the realm" "Sent with the server\'s admin menu."'
# mangos_string 169 (LANG_MAIL_SENT) as the world database's migrations leave it: English, and
# German in content_loc3 (20230317162803_world.sql); the other locales empty.
MAIL_SENT = ["Mail sent to %s", "", "", "E-Mail an %s gesendet"]

# The stand-in for the mariadb client: the agent's queries answered from characters.json, each call
# written down (argv and environment) in calls.jsonl.
STUB = r'''#!@PYTHON@ -I
import json, os, re, sys, time, unicodedata
here = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(here, "calls.jsonl"), "a", encoding="utf-8") as f:
    seen = {"argv": sys.argv[1:], "pwd": os.environ.get("MYSQL_PWD"),
            "env": sorted(k for k in os.environ if k in ("INILLA_ADMIN_CODE", "DB_PASSWORD"))}
    f.write(json.dumps(seen) + "\n")
with open(os.path.join(here, "characters.json"), encoding="utf-8") as f:
    db = json.load(f)
if db.get("down"):
    sys.exit("ERROR 2002 (HY000): Can't connect to server on 'db' (115)")
time.sleep(db.get("sleep", 0))
sql = sys.argv[sys.argv.index("-e") + 1]
if sys.argv[sys.argv.index("-e") - 1] != "characters":
    if db.get("world_down") or "FROM mangos_string WHERE entry = 169" not in sql:
        sys.exit("ERROR 1146 (42S02): Table 'mangos.mangos_string' doesn't exist")
    texts = db["mail_sent"] + [""] * (9 - len(db["mail_sent"]))
    sys.stdout.write("\t".join(t.encode().hex().upper() for t in texts) + "\n")
    sys.exit()
rows = [c for c in db["characters"] if not ("deleted_time IS NULL" in sql and c.get("deleted"))]
if "online = 1" in sql:
    rows = [c for c in rows if c["online"]]
names = [bytes.fromhex(h).decode() for h in re.findall(r"UNHEX\('([0-9a-f]*)'\)", sql)]
if " IN (" in sql:
    wanted = {n.casefold() for n in names}
    rows = [c for c in rows if c["name"].casefold() in wanted]
# utf8_general_ci's order: accents and case left out.
rows.sort(key=lambda c: unicodedata.normalize("NFKD", c["name"]).encode("ascii", "ignore").lower())
cols = ["name", "level", "race", "class"] if sql.startswith("SELECT name, level") else ["name"]
sys.stdout.buffer.write("".join("\t".join(str(c[k]) for k in cols) + "\n" for c in rows).encode())
'''


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class FakeConsole:
    """mangosd's console end of console.fifo, as inilla-mangosd wires it: a line read, then the log
    noise of the world tick, the command's answer and `mangos>` appended to console.log."""

    def __init__(self, run_dir):
        self.fifo = os.path.join(run_dir, "console.fifo")
        self.log = os.path.join(run_dir, "console.log")
        os.mkfifo(self.fifo, 0o600)
        open(self.log, "wb").close()
        # Read-write, as `exec 3<>`: writers never block, never see "no reader".
        self.fd = os.open(self.fifo, os.O_RDWR | os.O_NONBLOCK)
        self.commands = []
        # The names mangosd itself knows (send money looks them up in its own cache), and the
        # LANG_MAIL_SENT it answers in (the realm's DBC language).
        self.cache = {"Thrall", "Jaina", "Élise", "Anna", "Bea", "Cid", "Dag", "Eve", "Fay"}
        self.mail_sent = "Mail sent to %s"
        # Name -> another thread's log line, printed between that answer's text and its CRLF (two
        # printf calls in CliHandler::SendSysMessage).
        self.glue = {}
        self.delay = 0.02
        # Command prefix -> seconds before mangosd gets to it (None: never; "next": once the next
        # command is written), the queue behind it.
        self.hold = {}
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def stop(self):
        self.stopped.set()
        self.thread.join(5)
        os.close(self.fd)

    def run(self):
        pending = b""
        while not self.stopped.is_set():
            if not select.select([self.fd], [], [], 0.05)[0]:
                continue
            with contextlib.suppress(BlockingIOError):
                pending += os.read(self.fd, 4096)
            while b"\n" in pending:
                line, pending = pending.split(b"\n", 1)
                self.answer(line.decode())

    def append(self, text):
        with open(self.log, "ab") as f:
            f.write(text.encode())

    def answer(self, line):
        self.commands.append(line)
        for prefix, wait in list(self.hold.items()):
            if not line.startswith(prefix):
                continue
            if wait == "next":
                select.select([self.fd], [], [], 5)
            elif wait is None or self.stopped.wait(wait):
                return
        time.sleep(self.delay)
        self.append("2026-10-17 12:00:01 Map 0 (Eastern Kingdoms): 2 players in the grid\n")
        if line == "reload config":
            reply = (
                "Re-Loading config settings...\nVMap support included. LineOfSight:0, "
                "getHeight:0, indoorCheck:0\n* Anticrash  : 0\nWorld config settings reloaded.\r\n"
            )
        elif line.startswith("announce "):
            reply = ""
        elif line.startswith("send money "):
            name = line.split(" ")[2]
            # normalizePlayerName (the first letter upper case, the rest lower), then mangosd's own
            # name map, which is exact: Weiss is not Weiß.
            shown = name[:1].upper() + name[1:].lower()
            if shown in {n[:1].upper() + n[1:].lower() for n in self.cache}:
                reply = self.mail_sent.replace("%s", shown) + self.glue.pop(name, "") + "\r\n"
            else:
                reply = "Player not found!\r\n"
        else:
            reply = "There is no such command.\r\n"
        self.append(reply + "mangos>")


class Folders(unittest.TestCase):
    """The agent's folders and environment, and the stub database."""

    speed_patch = False

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="vanillish-admin-test-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        for name in ("run", "admin", "proc", "bin"):
            os.mkdir(os.path.join(self.dir, name))
        self.run_dir = os.path.join(self.dir, "run")
        self.rates = os.path.join(self.dir, "admin", "rates.conf")
        self.base = os.path.join(self.run_dir, "mangosd.base.conf")
        self.live = os.path.join(self.run_dir, "mangosd.conf")
        self.env = {
            "INILLA_ADMIN_RUN_DIR": self.run_dir,
            "INILLA_ADMIN_STATE_DIR": os.path.join(self.dir, "admin"),
            "INILLA_ADMIN_PATCHES": os.path.join(self.dir, "vanillish-patches"),
            "INILLA_ADMIN_PROC_NET": os.path.join(self.dir, "proc"),
            "PATH": os.path.join(self.dir, "bin") + os.pathsep + os.environ.get("PATH", ""),
            "DB_HOST": "db",
            "DB_PASSWORD": DB_PASSWORD,
            "INILLA_ADMIN_CODE": CODE,
        }
        patcher = mock.patch.dict(os.environ, self.env)
        patcher.start()
        self.addCleanup(patcher.stop)
        for path in (self.base, self.live):
            with open(path, "wb") as f:
                f.write(BASE)
        with open(self.env["INILLA_ADMIN_PATCHES"], "w") as f:
            f.write("0001-move-speed\n" if self.speed_patch else "")
        self.world(True)
        stub = os.path.join(self.dir, "bin", "mariadb")
        with open(stub, "w") as f:
            f.write(STUB.replace("@PYTHON@", sys.executable))
        os.chmod(stub, 0o755)
        self.database(CHARACTERS)

    def world(self, up):
        with open(os.path.join(self.dir, "proc", "tcp"), "w") as f:
            f.write(TCP_HEAD + TCP_8091 + (TCP_8085 if up else ""))

    def database(self, characters, **state):
        state.setdefault("mail_sent", MAIL_SENT)
        with open(os.path.join(self.dir, "bin", "characters.json"), "w", encoding="utf-8") as f:
            json.dump(dict(characters=characters, **state), f)

    def db_calls(self, database=None):
        """The stub's calls, on one database or all of them."""
        path = os.path.join(self.dir, "bin", "calls.jsonl")
        if not os.path.exists(path):
            return []
        with open(path, encoding="utf-8") as f:
            calls = [json.loads(line) for line in f]
        return [c for c in calls if database in (None, c["argv"][c["argv"].index("-e") - 1])]

    def read(self, path):
        with open(path, "rb") as f:
            return f.read()

    def write_rates(self, text):
        with open(self.rates, "w", encoding="utf-8") as f:
            f.write(text)


class Api(Folders):
    """The agent's HTTP API in this process, with a fake console, its timings cut short."""

    times = dict(
        WORLD_GRACE=0, LOCK_WAIT=0.5, CONSOLE_TIMEOUT=0.6, SUSPECT_WAIT=1.5, POLL=0.02,
        MIN_COMMAND=0.2, DB_TIMEOUT=3,
    )

    def setUp(self):
        super().setUp()
        for name, value in self.times.items():
            patcher = mock.patch.object(admin, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        # The agent logs a request after its reply: a test waits for the line (wait_logs).
        self.logs = []
        self.logged = threading.Condition()
        patcher = mock.patch.object(admin, "log", self.log)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.console = FakeConsole(self.run_dir)
        self.addCleanup(self.console.stop)
        self.pending = os.path.join(self.run_dir, "console.pending")
        self.server = admin.make_server(("127.0.0.1", 0), CODE)
        self.port = self.server.server_address[1]
        thread = threading.Thread(
            target=self.server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True
        )
        thread.start()
        self.addCleanup(self.stop_server)

    def log(self, line):
        with self.logged:
            self.logs.append(line)
            self.logged.notify_all()

    def wait_logs(self, n, timeout=5):
        """The agent's log lines once there are n of them (or at the timeout, as they are)."""
        with self.logged:
            self.logged.wait_for(lambda: len(self.logs) >= n, timeout)
            return list(self.logs)

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.server.agent.stop.set()

    def call(self, method, path, body=None, code=CODE, raw=None):
        """(status, parsed reply, raw reply bytes)."""
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        headers = {"Content-Type": "application/json"}
        if code is not None:
            headers["Authorization"] = f"Bearer {code}"
        data = raw if raw is not None else (None if body is None else json.dumps(body).encode())
        if method == "GET":
            headers["Content-Length"] = "0"
        conn.request(method, path, body=data, headers=headers)
        r = conn.getresponse()
        payload = r.read()
        conn.close()
        return r.status, json.loads(payload), payload

    def post(self, path, body):
        status, doc, _ = self.call("POST", path, body)
        return status, doc

    def tearDown(self):
        # No secret in any log line.
        for line in self.wait_logs(0):
            self.assertNotIn(normal(CODE), line.replace("-", ""))
            self.assertNotIn(DB_PASSWORD, line)


def normal(code):
    return code.replace("-", "").upper()


# ---- the code ------------------------------------------------------------------------------------


class Code(unittest.TestCase):
    def test_the_setups_form(self):
        for code in ("ABCD-EFGH-JKLM-NPQR", "ABCDEFGHJKLMNPQR23", "WXYZ-2345-6789-STUV"):
            self.assertTrue(admin.usable_code(code), code)
        # Its 32 symbols leave out 0, 1, I and O.
        for code in ("ABCD-EFGH-JKLM-NPQ", "ABCD-EFGH-JKLM-NPQO", "ABCD-EFGH-JKLM-NPQI"):
            self.assertFalse(admin.usable_code(code), code)

    def test_hex_digits(self):
        for code in ("0123456789abcdef0123", "a1" * 10, "FFFF-0000-FFFF-0000-1111"):
            self.assertTrue(admin.usable_code(code), code)
        for code in ("0123456789abcdef012", "a1" * 8, "0123456789abcdef012g"):
            self.assertFalse(admin.usable_code(code), code)

    def test_a_word_is_refused(self):
        for code in (
            "VANILLISHADMIN2026", "vanillishadmin2026", "VanillishServer2024", "PASSWORDPASSWORD",
        ):
            self.assertFalse(admin.usable_code(code), code)

    def test_the_example_is_refused(self):
        for code in ("change-me-admin-code-0000", "CHANGEME-ABCD-EFGH-JKLM", "changemeabcdefghjk"):
            self.assertFalse(admin.usable_code(code), code)

    def test_normalised(self):
        for code in ("abcd efgh jklm npqr", " abcd-EFGH\tjklm-npqr\r\n", "abcdefghjklmnpqr"):
            self.assertTrue(admin.usable_code(code), repr(code))
        self.assertEqual(admin.normalized_code(" ab-cd\t12\r\n"), "ABCD12")
        for code in (
            "", None, "----", "ABCD-EFGH-JKLM-NPQß", "ÄBCD-EFGH-JKLM-NPQR", "ABCD_EFGH_JKLM_NPQR",
            "ABCD\x0bEFGH-JKLM-NPQR",
        ):
            self.assertFalse(admin.usable_code(code), repr(code))


# ---- render --------------------------------------------------------------------------------------


class Render(Folders):
    def render(self, **env):
        """`vanillish-admin render` as inilla-mangosd runs it: (exit status, its log)."""
        r = subprocess.run(
            [sys.executable, "-I", AGENT, "render"],
            env=dict(os.environ, **env), capture_output=True, text=True, timeout=30,
        )
        return r.returncode, r.stdout + r.stderr

    def expect(self, *swaps, extra=b""):
        """BASE with lines swapped (old -> new) and lines appended."""
        out = BASE
        for old, new in swaps:
            self.assertIn(old, out)
            out = out.replace(old, new, 1)
        return out + extra

    def test_in_place_append_and_junk(self):
        with open(self.env["INILLA_ADMIN_PATCHES"], "w") as f:
            f.write("0001-move-speed\n")
        self.write_rates(
            "# the menu's rates\n"
            "Rate.XP.Kill = 2.50\r\n"
            "Rate.XP.Quest=2\n"
            "Rate.Drop.Item.Uncommon = 3.456\n"
            "Rate.MoveSpeed.Player = 1.2\n"
            "Rate.Drop.Item.Poor = 007\n"
            "Rate.Drop.Money = 1e2\n"
            "Rate.Drop.Item.Rare = ١٢\n"
            "Rate.Drop.Item.Epic = 25\n"
            "Rate.Drop.Item.Normal = -1\n"
            "Rate.Drop.Item.Legendary = nan\n"
            "Rate.Drop.Item.Referenced = 2 # inline\n"
            "rate.xp.explore = 2\n"
            "Rate.Health = 2\n"
            "a line with no equals sign\n"
            "\n"
        )
        status, out = self.render()
        self.assertEqual(status, 0, out)
        self.assertEqual(
            self.read(self.live),
            self.expect(
                (b"Rate.XP.Kill = 1\n", b"Rate.XP.Kill = 2.5\n"),
                (b"  rate.xp.quest=1\n", b"Rate.XP.Quest = 2\n"),
                (b"Rate.Drop.Item.Uncommon = 1\n", b"Rate.Drop.Item.Uncommon = 3.46\n"),
                (b"Rate.Drop.Item.Poor = 1\n", b"Rate.Drop.Item.Poor = 7\n"),
                extra=b"Rate.MoveSpeed.Player = 1.2\n",
            ),
        )
        self.assertEqual(self.read(self.base), BASE)
        self.assertEqual(out.count("dropped"), 9, out)
        self.assertIn("Rate.XP.Kill = 2.5 from the admin menu (the server's own: 1)", out)
        self.assertFalse(os.path.exists(os.path.join(self.run_dir, "mangosd.conf.admin-tmp")))

    def test_no_overrides_is_the_base_byte_for_byte(self):
        status, out = self.render()
        self.assertEqual((status, self.read(self.live)), (0, BASE), out)
        self.write_rates("# nothing yet\n")
        self.assertEqual(self.render()[0], 0)
        self.assertEqual(self.read(self.live), BASE)

    def test_speed_needs_its_patch(self):
        self.write_rates("Rate.MoveSpeed.Player = 1.5\nRate.XP.Explore = 3\n")
        status, out = self.render()
        self.assertEqual(status, 0, out)
        self.assertEqual(
            self.read(self.live), self.expect((b"Rate.XP.Explore = 1\n", b"Rate.XP.Explore = 3\n"))
        )
        self.assertIn("this vmangos lacks it", out)

    def test_admin_off_applies_nothing(self):
        self.write_rates("Rate.XP.Kill = 3\n")
        status, out = self.render(INILLA_ADMIN_CODE="change-me-please-now")
        self.assertEqual((status, self.read(self.live)), (0, BASE), out)
        self.assertIn("1 admin override(s) not applied", out)
        self.assertNotIn("change-me", out)

    def test_a_failure_exits_1_and_leaves_the_conf(self):
        os.unlink(self.base)
        with open(self.live, "wb") as f:
            f.write(b"untouched\n")
        status, out = self.render()
        self.assertEqual(status, 1, out)
        self.assertIn("render failed", out)
        self.assertEqual(self.read(self.live), b"untouched\n")

    def test_the_same_lines_as_conf_set(self):
        """merge() is inilla-lib.sh's conf_set, one override after the other (on a base without
        CR: conf_set drops every CR)."""
        base = BASE.replace(b"\r", b"")
        overrides = {"Rate.XP.Kill": 2.5, "Rate.XP.Quest": 2.0, admin.SPEED: 1.2}
        path = os.path.join(self.dir, "conf_set.conf")
        with open(path, "wb") as f:
            f.write(base)
        for key, value in overrides.items():
            subprocess.run(
                ["bash", "-c", '. "$0" && conf_set "$1" "$2" "$3"',
                 os.path.join(BIN, "inilla-lib.sh"), path, key, admin.fmt(value)],
                check=True, timeout=30,
            )
        self.assertEqual(admin.merge(base, overrides), self.read(path))

    def test_what_mangosd_reads(self):
        values = admin.conf_values(
            b'A = 1\nA = 2\n  B\t= "x y" # c\n#C = 3\n[D] = 4\nE =\nF = 5\r6\nG.H = 1.5abc\n=7\n'
        )
        self.assertEqual(values, {"A": "1", "B": "x y ", "F": "5", "G.H": "1.5abc"})
        rates = admin.rate_values(BASE)
        self.assertEqual(rates["Rate.XP.Kill"], 1)
        self.assertEqual(rates["Rate.XP.Quest"], 1)
        self.assertEqual(rates["Rate.Drop.Money"], 1.5)
        self.assertEqual(rates[admin.SPEED], 1)


# ---- status, auth, the world ---------------------------------------------------------------------


class Status(Api):
    def test_status(self):
        status, doc, _ = self.call("GET", "/status")
        self.assertEqual(status, 200)
        self.assertEqual(
            (doc["ok"], doc["world"], doc["overrides"], doc["players_ok"]), (True, "up", 0, True)
        )
        self.assertEqual(list(doc["rates"]), list(admin.XP + admin.LOOT) + ["Rate.Drop.Money"])
        kill = {"value": 1, "server": 1, "min": 0, "max": 20}
        self.assertEqual(doc["rates"]["Rate.XP.Kill"], kill)
        self.assertEqual(doc["rates"]["Rate.Drop.Money"]["server"], 1.5)
        self.assertEqual(
            doc["players"],
            [
                {"name": "Élise", "level": 5, "race": 1, "class": 5},
                {"name": "Jaina", "level": 20, "race": 1, "class": 8},
                {"name": "Thrall", "level": 12, "race": 2, "class": 1},
            ],
        )
        (call,) = self.db_calls()
        self.assertEqual(call["pwd"], DB_PASSWORD)
        self.assertEqual(call["env"], [])
        self.assertFalse(any(DB_PASSWORD in a or "--password" in a for a in call["argv"]))
        self.assertIn("--default-character-set=utf8mb4", call["argv"])
        self.assertEqual(self.wait_logs(1), ["GET status 200"])

    def test_database_down(self):
        self.database(CHARACTERS, down=True)
        status, doc, _ = self.call("GET", "/status")
        self.assertEqual((status, doc["players"], doc["players_ok"]), (200, [], False))

    def test_world_starting(self):
        self.world(False)
        self.assertEqual(self.call("GET", "/status")[1]["world"], "starting")
        for path, body in (
            ("/rates", {"rates": {"Rate.XP.Kill": 2}, "announce": False}),
            ("/rates/reset", {"announce": False}),
            ("/gold", {"id": "g-1", "to": ["Thrall"], "copper": 100}),
        ):
            status, doc = self.post(path, body)
            self.assertEqual((status, doc["error"]), (503, "world_starting"), path)
        self.assertEqual(self.console.commands, [])
        self.assertFalse(os.path.exists(self.rates))

    def test_the_world_counts_once_it_has_listened_a_while(self):
        self.world(False)
        with mock.patch.object(admin, "WORLD_GRACE", 0.4):
            self.assertEqual(self.call("GET", "/status")[1]["world"], "starting")
            self.world(True)
            self.assertEqual(self.call("GET", "/status")[1]["world"], "starting")
            time.sleep(0.6)
            self.assertEqual(self.call("GET", "/status")[1]["world"], "up")
            self.world(False)
            self.assertEqual(self.call("GET", "/status")[1]["world"], "starting")


class Speed(Api):
    speed_patch = True

    def test_speed_with_its_patch(self):
        doc = self.call("GET", "/status")[1]
        self.assertEqual(doc["rates"][admin.SPEED], {"value": 1, "server": 1, "min": 0.5, "max": 3})
        for value in (0.4, 3.5):
            status, doc = self.post("/rates", {"rates": {admin.SPEED: value}})
            self.assertEqual((status, doc["error"]), (400, "out_of_range"))
        status, doc = self.post("/rates", {"rates": {admin.SPEED: 1.2}, "announce": True})
        self.assertEqual((status, doc["changed"]), (200, [admin.SPEED]), doc)
        self.assertTrue(
            self.read(self.live).endswith(b"mmap.enabled = 0\nRate.MoveSpeed.Player = 1.2\n")
        )
        self.assertEqual(
            self.console.commands,
            ["reload config", "announce Server rates: XP x1, loot x1, gold x1.5, speed x1.2"],
        )


class Auth(Api):
    def test_the_code(self):
        wrong = (
            None, "", "WRONG-CODE-WRONG-CODE", CODE + "X", CODE[:-1], CODE + "\x0b",
            "ÄBCD-EFGH-JKLM-NPQR",
        )
        for code in wrong:
            status, doc, _ = self.call("GET", "/status", code=code)
            self.assertEqual((status, doc["error"]), (401, "admin_auth"), repr(code))
        status, doc, _ = self.call("POST", "/nowhere", {}, code=None)
        self.assertEqual((status, doc["error"]), (401, "admin_auth"))
        status, doc, _ = self.call("POST", "/nowhere", {})
        self.assertEqual((status, doc["error"]), (404, "not_found"))
        self.assertEqual(self.db_calls(), [])
        self.assertIn("POST unknown 404 not_found", self.wait_logs(len(wrong) + 2))
        # Read normalised, as the data host sends it: case, dashes and spaces aside.
        for code in (normal(CODE), CODE.lower(), " abcd efgh-JKLM\tnpqr "):
            self.assertEqual(self.call("GET", "/status", code=code)[0], 200, repr(code))
        with socket.create_connection(("127.0.0.1", self.port), timeout=5) as s:
            s.sendall(b"GET /status HTTP/1.1\r\nAuthorization: Basic %s\r\n\r\n" % CODE.encode())
            self.assertTrue(s.recv(4096).startswith(b"HTTP/1.1 401 "))

    def test_the_code_comes_before_the_body(self):
        with socket.create_connection(("127.0.0.1", self.port), timeout=5) as s:
            s.sendall(
                b"POST /gold HTTP/1.1\r\nHost: mangosd\r\nAuthorization: Bearer nope\r\n"
                b"Content-Type: application/json\r\nContent-Length: 1000\r\n\r\n"
            )
            started = time.monotonic()
            head = s.recv(4096)
        self.assertTrue(head.startswith(b"HTTP/1.1 401 "), head)
        self.assertLess(time.monotonic() - started, 0.9)

    def test_bodies(self):
        cases = (
            (b"not json", "bad_request"),
            (b"[1]", "bad_request"),
            (b'{"rates": {"Rate.XP.Kill": NaN}}', "bad_request"),
            (b'{"rates": {"Rate.XP.Kill": Infinity}}', "bad_request"),
            (b'{"rates": {"Rate.XP.Kill": -Infinity}}', "bad_request"),
            (b'{"rates": {"Rate.XP.Kill": 1e400}}', "out_of_range"),
            (b"\xff\xfe{}", "bad_request"),
            (b"[" * 60000, "bad_request"),
        )
        for raw, error in cases:
            status, doc, _ = self.call("POST", "/rates", raw=raw)
            self.assertEqual((status, doc["error"]), (400, error), raw[:40])
        # Over 64 KiB: refused from the head, the body never read.
        with socket.create_connection(("127.0.0.1", self.port), timeout=5) as s:
            s.sendall(
                b"POST /rates HTTP/1.1\r\nAuthorization: Bearer " + CODE.encode()
                + b"\r\nContent-Length: 65537\r\n\r\n"
            )
            reply = b"".join(iter(lambda: s.recv(4096), b""))
        self.assertTrue(reply.startswith(b"HTTP/1.1 400 "), reply)
        self.assertIn(b"over 64 KiB", reply)
        self.assertEqual(self.console.commands, [])


class Busy(Api):
    times = dict(Api.times, MAX_REQUESTS=1)

    def test_at_most_n_at_once(self):
        with mock.patch.object(admin.Handler, "timeout", 2):
            hold = socket.create_connection(("127.0.0.1", self.port), timeout=5)
            hold.sendall(
                b"POST /rates HTTP/1.1\r\nHost: mangosd\r\nAuthorization: Bearer "
                + CODE.encode() + b"\r\nContent-Length: 10\r\n\r\n"
            )
            time.sleep(0.2)
            status, doc, _ = self.call("GET", "/status")
            self.assertEqual((status, doc["error"]), (503, "busy"))
            hold.close()
            for _ in range(50):
                time.sleep(0.1)
                if self.call("GET", "/status")[0] == 200:
                    break
            else:
                self.fail("the slot never came back")


# ---- rates ---------------------------------------------------------------------------------------


class Rates(Api):
    def test_apply_reload_announce(self):
        body = {
            "id": "rates-1",
            "rates": {"Rate.XP.Kill": 2, "Rate.XP.Quest": 2, "Rate.XP.Explore": 2.0,
                      "Rate.Drop.Item.Rare": 1.5},
            "announce": True,
        }
        status, doc = self.post("/rates", body)
        self.assertEqual(status, 200, doc)
        keys = ["Rate.XP.Kill", "Rate.XP.Quest", "Rate.XP.Explore", "Rate.Drop.Item.Rare"]
        self.assertEqual(doc["changed"], keys)
        kill = {"value": 2, "server": 1, "min": 0, "max": 20}
        self.assertEqual(doc["rates"]["Rate.XP.Kill"], kill)
        self.assertEqual(doc["rates"]["Rate.Drop.Item.Rare"]["value"], 1.5)
        self.assertEqual((doc["overrides"], doc["world"], doc["players_ok"]), (4, "up", True))
        self.assertEqual(
            self.read(self.rates).decode().splitlines()[2:],
            [f"{k} = {v}" for k, v in zip(keys, ("2", "2", "2", "1.5"))],
        )
        live = self.read(self.live)
        for line in (b"\nRate.XP.Kill = 2\nRate.XP.Kill = 7\nRate.XP.Quest = 2\n",
                     b"\nRate.Drop.Item.Rare = 1.5\n", b"Odd.Line = x\r\n"):
            self.assertIn(line, live)
        self.assertEqual(
            self.console.commands,
            ["reload config", "announce Server rates: XP x2, loot x1 to x1.5, gold x1.5"],
        )
        self.assertEqual(
            self.wait_logs(1),
            ["POST rates 200 changed Rate.XP.Kill=2, Rate.XP.Quest=2, Rate.XP.Explore=2, "
             "Rate.Drop.Item.Rare=1.5, announced"],
        )

    def test_rounding_server_value_and_no_op(self):
        status, doc = self.post("/rates", {"rates": {"Rate.Drop.Money": 1.5}})
        self.assertEqual((status, doc["changed"]), (200, []))
        self.assertFalse(os.path.exists(self.rates))
        status, doc = self.post("/rates", {"rates": {"Rate.Drop.Money": 1.234, "Rate.XP.Kill": 0}})
        self.assertEqual((status, doc["changed"]), (200, ["Rate.XP.Kill", "Rate.Drop.Money"]))
        self.assertIn(b"\nRate.Drop.Money = 1.23\n", self.read(self.live))
        self.assertIn(b"\nRate.XP.Kill = 0\n", self.read(self.live))
        self.assertEqual(self.console.commands, ["reload config"])
        # The same again: nothing to do, no reload.
        status, doc = self.post("/rates", {"rates": {"Rate.Drop.Money": 1.23}, "announce": True})
        self.assertEqual((status, doc["changed"]), (200, []))
        self.assertEqual(self.console.commands, ["reload config"])
        # Back to the server's own value: the override goes.
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Kill": 1}})
        self.assertEqual((status, doc["changed"], doc["overrides"]), (200, ["Rate.XP.Kill"], 1))
        self.assertNotIn(b"Rate.XP.Kill", self.read(self.rates))
        self.assertIn(b"\nRate.XP.Kill = 1\nRate.XP.Kill = 7\n", self.read(self.live))

    def test_validation_is_all_or_nothing(self):
        cases = (
            ({"Rate.XP.Kill": True}, "bad_request", "Rate.XP.Kill"),
            ({"Rate.XP.Kill": "2"}, "bad_request", "Rate.XP.Kill"),
            ({"Rate.XP.Kill": None}, "bad_request", "Rate.XP.Kill"),
            ({"Rate.XP.Kill": 20.01}, "out_of_range", "Rate.XP.Kill"),
            ({"Rate.XP.Kill": -0.5}, "out_of_range", "Rate.XP.Kill"),
            ({"Rate.XP.Kill": 10 ** 400}, "out_of_range", "Rate.XP.Kill"),
            ({"rate.xp.kill": 2}, "unknown_key", "rate.xp.kill"),
            ({"Rate.Health": 2}, "unknown_key", "Rate.Health"),
            ({admin.SPEED: 1.2}, "unknown_key", admin.SPEED),
            ({"Rate.XP.Kill": 2, "Rate.XP.Quest": 99}, "out_of_range", "Rate.XP.Quest"),
        )
        for rates, error, key in cases:
            status, doc = self.post("/rates", {"rates": rates, "announce": True})
            self.assertEqual((status, doc["error"]), (400, error), rates)
            self.assertIn(key, doc["message"])
        for body in (
            {"rates": [1]},
            {"announce": True},
            {"rates": {}, "announce": "yes"},
            {"rates": {}, "id": "not an id"},
            {"rates": {}, "id": "x" * 65},
            {"rates": {}, "id": 7},
        ):
            status, doc = self.post("/rates", body)
            self.assertEqual((status, doc["error"]), (400, "bad_request"), body)
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Kill": 20, "Rate.XP.Quest": 0}})
        self.assertEqual(status, 200)
        self.assertEqual(self.console.commands, ["reload config"])

    def test_reset(self):
        self.post("/rates", {"rates": {"Rate.XP.Kill": 3, "Rate.Drop.Money": 2}})
        status, doc = self.post("/rates/reset", {"id": "reset-1", "announce": True})
        self.assertEqual(
            (status, doc["changed"], doc["overrides"]), (200, ["Rate.XP.Kill", "Rate.Drop.Money"], 0)
        )
        self.assertFalse(os.path.exists(self.rates))
        self.assertEqual(self.read(self.live), BASE)
        self.assertEqual(
            self.console.commands,
            ["reload config", "reload config", "announce Server rates: XP x1, loot x1, gold x1.5"],
        )
        status, doc = self.post("/rates/reset", {"announce": True})
        self.assertEqual((status, doc["changed"]), (200, []))
        self.assertEqual(len(self.console.commands), 3)

    def test_a_reload_that_times_out(self):
        self.console.hold["reload config"] = 1.0
        body = {"id": "slow-1", "rates": {"Rate.XP.Kill": 2}, "announce": True}
        status, doc = self.post("/rates", body)
        self.assertEqual((status, doc["error"]), (503, "console_timeout"))
        self.assertIn(b"Rate.XP.Kill = 2", self.read(self.rates))
        # The same id: the same answer, nothing run again.
        self.assertEqual(self.post("/rates", body), (status, doc))
        self.assertEqual(self.console.commands, ["reload config"])
        # The next change waits for the late prompt, then goes on.
        del self.console.hold["reload config"]
        status, doc = self.post("/rates", {"id": "next-1", "rates": {"Rate.XP.Kill": 3}})
        self.assertEqual((status, doc["changed"]), (200, ["Rate.XP.Kill"]), doc)
        self.assertEqual(self.console.commands, ["reload config", "reload config"])

    def test_a_console_that_never_answers(self):
        self.console.hold["reload config"] = None
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Kill": 2}})
        self.assertEqual((status, doc["error"]), (503, "console_timeout"))
        before = self.read(self.rates)
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Kill": 4}})
        self.assertEqual((status, doc["error"]), (503, "console_busy"))
        self.assertEqual(self.read(self.rates), before)
        self.assertEqual(self.console.commands, ["reload config"])

    def test_console_lock_held(self):
        with open(os.path.join(self.run_dir, "console.lock"), "w") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            status, doc = self.post("/rates", {"rates": {"Rate.XP.Kill": 2}})
            self.assertEqual((status, doc["error"]), (503, "console_busy"))
            self.assertFalse(os.path.exists(self.rates))
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Kill": 2}})
        self.assertEqual(status, 200)

    def test_a_rate_this_image_lacks_is_kept(self):
        """The speed line on an image without its patch: unapplied, kept for one with it, and
        counted, so the app offers "Server defaults" for it."""
        self.write_rates("Rate.MoveSpeed.Player = 1.3\nRate.XP.Kill = 2\n")
        self.assertEqual(self.call("GET", "/status")[1]["overrides"], 2)
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Quest": 3}})
        changed = ["Rate.XP.Kill", "Rate.XP.Quest"]
        self.assertEqual((status, doc["changed"], doc["overrides"]), (200, changed, 3), doc)
        kept = ["Rate.XP.Kill = 2", "Rate.XP.Quest = 3", "Rate.MoveSpeed.Player = 1.3"]
        self.assertEqual(self.read(self.rates).decode().splitlines()[2:], kept)
        self.assertNotIn(b"MoveSpeed", self.read(self.live))
        status, doc = self.post("/rates/reset", {})
        self.assertEqual((status, doc["overrides"]), (200, 0))
        self.assertFalse(os.path.exists(self.rates))

    def test_server_defaults_drop_a_lone_kept_rate(self):
        self.write_rates("Rate.MoveSpeed.Player = 1.3\n")
        self.assertEqual(self.call("GET", "/status")[1]["overrides"], 1)
        status, doc = self.post("/rates/reset", {"announce": True})
        self.assertEqual((status, doc["changed"], doc["overrides"]), (200, [], 0), doc)
        self.assertFalse(os.path.exists(self.rates))
        self.assertEqual((self.read(self.live), self.console.commands), (BASE, []))
        # A change that leaves it alone keeps it.
        self.write_rates("Rate.MoveSpeed.Player = 1.3\n")
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Kill": 1}})
        self.assertEqual((status, doc["changed"], doc["overrides"]), (200, [], 1), doc)
        self.assertEqual(self.read(self.rates), b"Rate.MoveSpeed.Player = 1.3\n")

    def test_a_render_that_fails_changes_nothing(self):
        self.post("/rates", {"rates": {"Rate.XP.Kill": 2}})
        before = self.read(self.rates)
        os.unlink(self.base)
        status, doc = self.post("/rates", {"rates": {"Rate.XP.Quest": 3}})
        self.assertEqual((status, doc["error"]), (500, "internal"))
        self.assertEqual(self.read(self.rates), before)
        self.assertEqual(self.console.commands, ["reload config"])


# ---- gold ----------------------------------------------------------------------------------------


def money(name, copper):
    return f"send money {name} {GIFT} {copper}"


class Gold(Api):
    def test_sent_missing_unsure(self):
        to = ["thrall", "Jaina", "Nobody", "Bob", "THRALL", "Gone"]
        body = {"id": "gold-1", "to": to, "copper": 10000}
        status, doc = self.post("/gold", body)
        self.assertEqual(status, 200, doc)
        self.assertEqual(
            doc,
            {"ok": True, "sent": ["Thrall", "Jaina"], "missing": ["Nobody", "Gone"],
             "unsure": ["Bob"], "not_sent": []},
        )
        self.assertEqual(
            self.console.commands, [money(n, 10000) for n in ("Thrall", "Jaina", "Bob")]
        )
        (call,) = self.db_calls("characters")
        sql = call["argv"][call["argv"].index("-e") + 1]
        for name in ("hrall", "HRALL", "Jaina", "Nobody"):
            self.assertNotIn(name, sql)
        self.assertIn("CONVERT(UNHEX('746872616c6c') USING utf8mb4)", sql)
        self.assertIn("deleted_time IS NULL", sql)
        self.assertEqual(
            self.wait_logs(1),
            ["POST gold 200 10000 copper to 3: sent 2, unsure 1, not sent 0, missing 2"],
        )

    def test_an_id_is_answered_once(self):
        body = {"id": "gold-2", "to": ["Thrall"], "copper": 500}
        first = self.call("POST", "/gold", body)
        self.assertEqual(self.call("POST", "/gold", body), first)
        self.assertEqual(self.call("POST", "/gold", dict(body, copper=1)), first)
        self.assertEqual(self.console.commands, [money("Thrall", 500)])
        self.assertEqual(self.post("/gold", dict(body, id="gold-3"))[1]["sent"], ["Thrall"])
        self.assertEqual(len(self.console.commands), 2)

    def test_everyone_online(self):
        status, doc = self.post("/gold", {"id": "all-1", "to": "online", "copper": 1})
        self.assertEqual(
            (status, doc["sent"], doc["missing"]), (200, ["Élise", "Jaina", "Thrall"], [])
        )
        self.assertEqual(self.console.commands[0], money("Élise", 1))

    def test_validation(self):
        good = {"id": "v-1", "to": ["Thrall"], "copper": 100}
        for change, error in (
            ({"id": None}, "bad_request"),
            ({"id": ""}, "bad_request"),
            ({"copper": True}, "bad_request"),
            ({"copper": 1.5}, "bad_request"),
            ({"copper": "100"}, "bad_request"),
            ({"copper": 0}, "out_of_range"),
            ({"copper": 1_000_000_001}, "out_of_range"),
            ({"to": []}, "bad_request"),
            ({"to": "everyone"}, "bad_request"),
            ({"to": ["Bob\n"]}, "bad_request"),
            ({"to": ["B"]}, "bad_request"),
            ({"to": ["Abcdefghijklm"]}, "bad_request"),
            ({"to": ["Bo b"]}, "bad_request"),
            ({"to": ["Bob1"]}, "bad_request"),
            ({"to": ["Ⅻx"]}, "bad_request"),
            ({"to": ["Bob'"]}, "bad_request"),
            ({"to": [7]}, "bad_request"),
            ({"to": ["Bob"] * 51}, "bad_request"),
        ):
            body = {k: v for k, v in dict(good, **change).items() if v is not None}
            status, doc = self.post("/gold", body)
            self.assertEqual((status, doc["error"]), (400, error), change)
        self.assertEqual(self.console.commands, [])
        self.assertEqual(self.post("/gold", dict(good, copper=1_000_000_000))[0], 200)

    def test_the_deadline(self):
        names = ["Anna", "Bea", "Cid", "Dag", "Eve", "Fay"]
        self.database([{"name": n, "level": 1, "race": 1, "class": 1, "online": 1} for n in names])
        self.console.delay = 0.3
        with mock.patch.object(admin, "DEADLINE", 1.5):
            started = time.monotonic()
            status, doc = self.post("/gold", {"id": "slow-2", "to": names, "copper": 7})
        self.assertLess(time.monotonic() - started, 2.5)
        self.assertEqual(status, 200)
        self.assertTrue(doc["sent"] and doc["not_sent"], doc)
        self.assertEqual(sorted(doc["sent"] + doc["unsure"] + doc["not_sent"]), names)
        self.assertEqual(doc["not_sent"], names[-len(doc["not_sent"]):])

    def test_a_timeout_is_unsure_then_the_late_prompt_lets_the_next_go(self):
        self.console.hold[money("Jaina", 9)] = 1.0
        status, doc = self.post("/gold", {"id": "t-1", "to": ["Jaina", "Thrall"], "copper": 9})
        self.assertEqual(
            (status, doc["sent"], doc["unsure"], doc["not_sent"]), (200, ["Thrall"], ["Jaina"], [])
        )

    def test_a_late_answer_is_waited_for(self):
        """A command an earlier writer gave up on leaves its console.pending: the agent waits for
        that prompt before its own commands, so no answer lands on the wrong name. Bob is in the
        database, not in mangosd's cache."""
        self.console.hold["reload all"] = 0.6
        with open(self.console.fifo, "wb") as fifo:
            fifo.write(b"reload all\n")
        with open(self.pending, "w") as f:
            f.write("0\n")
        to = ["Thrall", "Jaina", "Bob"]
        status, doc = self.post("/gold", {"id": "late-1", "to": to, "copper": 9})
        self.assertEqual(
            (status, doc["sent"], doc["unsure"], doc["not_sent"]),
            (200, ["Thrall", "Jaina"], ["Bob"], []),
        )
        self.assertEqual(self.console.commands, ["reload all"] + [money(n, 9) for n in to])
        self.assertFalse(os.path.exists(self.pending))

    @mock.patch.object(admin, "CONSOLE_TIMEOUT", 2)
    def test_another_names_line_is_not_its_own(self):
        """'Mail sent to Alice', from a command no marker tells of, is not Al's answer (Al is
        unsure), and leaves nothing behind: once Al's own answer is in, the next send goes."""
        self.console.cache.update({"Al", "Alice"})
        self.database(CHARACTERS + [{"name": "Al", "level": 1, "race": 1, "class": 1, "online": 1}])
        self.console.hold[money("Alice", 9)] = "next"
        self.console.delay = 0.5
        with open(self.console.fifo, "wb") as fifo:
            fifo.write(money("Alice", 9).encode() + b"\n")
        status, doc = self.post("/gold", {"id": "al-1", "to": ["Al"], "copper": 9})
        self.assertEqual((status, doc["sent"], doc["unsure"]), (200, [], ["Al"]))
        self.assertFalse(os.path.exists(self.pending))
        end = time.monotonic() + 5
        while b"Mail sent to Al\r\nmangos>" not in self.read(self.console.log):
            self.assertLess(time.monotonic(), end, "Al's own answer never came")
            time.sleep(0.05)
        self.console.delay = 0.02
        status, doc = self.post("/gold", {"id": "al-2", "to": ["Thrall"], "copper": 9})
        self.assertEqual((status, doc["sent"]), (200, ["Thrall"]))
        self.assertFalse(os.path.exists(self.pending))

    @mock.patch.object(admin, "CONSOLE_TIMEOUT", 2)
    def test_weiss_is_not_weiss(self):
        """Weiß and Weiss, both online, are two characters to mangosd (and to the agent: str.lower,
        never casefold): both get their gold, and 'Mail sent to Weiss' is not Weiß's answer."""
        self.console.cache.update({"Weiß", "Weiss"})
        both = [{"name": n, "level": 1, "race": 1, "class": 1, "online": 1} for n in ("Weiß", "Weiss")]
        self.database(CHARACTERS + both)
        status, doc = self.post("/gold", {"id": "ss-1", "to": ["weiss", "Weiß", "WEISS"], "copper": 9})
        self.assertEqual(
            (status, doc["sent"], doc["unsure"], doc["missing"]), (200, ["Weiss", "Weiß"], [], []), doc
        )
        self.assertEqual(self.console.commands, [money("Weiss", 9), money("Weiß", 9)])
        # Weiss' answer, from a command no marker tells of, lands where Weiß's is read.
        self.console.hold[money("Weiss", 9)] = "next"
        self.console.delay = 0.5
        with open(self.console.fifo, "wb") as fifo:
            fifo.write(money("Weiss", 9).encode() + b"\n")
        status, doc = self.post("/gold", {"id": "ss-2", "to": ["Weiß"], "copper": 9})
        self.assertEqual((status, doc["sent"], doc["unsure"]), (200, [], ["Weiß"]), doc)
        self.assertIn("Mail sent to Weiss\r\nmangos>", self.read(self.console.log).decode())
        self.assertEqual(self.console.commands[2:], [money("Weiss", 9), money("Weiß", 9)])

    def test_the_name_as_mangosd_prints_it(self):
        """A McFly row (imported, or edited by hand) is answered 'Mail sent to Mcfly'."""
        self.console.cache.add("McFly")
        mcfly = {"name": "McFly", "level": 1, "race": 1, "class": 1, "online": 0}
        self.database(CHARACTERS + [mcfly])
        status, doc = self.post("/gold", {"id": "mc-1", "to": ["mcfly"], "copper": 9})
        self.assertEqual((status, doc["sent"], doc["unsure"]), (200, ["McFly"], []), doc)
        self.assertIn(b"Mail sent to Mcfly\r\n", self.read(self.console.log))
        self.assertEqual(self.console.commands, [money("McFly", 9)])
        self.assertFalse(os.path.exists(self.pending))

    def test_a_log_line_run_into_the_answer(self):
        """Another thread's log line between the answer's text and its CRLF: that name is unsure,
        and the next send still goes through."""
        self.console.glue["Thrall"] = "2026-10-17 12:00:02 Map 1 (Kalimdor): 1 player in the grid\n"
        status, doc = self.post("/gold", {"id": "glue-1", "to": ["Thrall", "Jaina"], "copper": 9})
        self.assertEqual((status, doc["sent"], doc["unsure"]), (200, ["Jaina"], ["Thrall"]), doc)
        self.assertFalse(os.path.exists(self.pending))
        status, doc = self.post("/gold", {"id": "glue-2", "to": ["Élise"], "copper": 9})
        self.assertEqual((status, doc["sent"]), (200, ["Élise"]))
        self.assertEqual(len(self.console.commands), 3)

    def test_the_realms_own_language(self):
        """A realm built from a German client answers in German (mangos_string's content_loc3);
        the world database is read once."""
        self.console.mail_sent = "E-Mail an %s gesendet"
        status, doc = self.post("/gold", {"id": "de-1", "to": ["Thrall"], "copper": 9})
        self.assertEqual((status, doc["sent"], doc["unsure"]), (200, ["Thrall"], []), doc)
        self.assertIn(b"E-Mail an Thrall gesendet\r\n", self.read(self.console.log))
        status, doc = self.post("/gold", {"id": "de-2", "to": ["Jaina", "Bob"], "copper": 9})
        self.assertEqual((status, doc["sent"], doc["unsure"]), (200, ["Jaina"], ["Bob"]), doc)
        (call,) = self.db_calls("mangos")
        self.assertEqual(call["pwd"], DB_PASSWORD)
        self.assertEqual(call["env"], [])
        self.assertIn("FROM mangos_string WHERE entry = 169", call["argv"][-1])

    def test_the_world_database_down_is_the_english_text(self):
        self.database(CHARACTERS, world_down=True)
        status, doc = self.post("/gold", {"id": "en-1", "to": ["Thrall"], "copper": 9})
        self.assertEqual((status, doc["sent"]), (200, ["Thrall"]), doc)
        self.console.mail_sent = "E-Mail an %s gesendet"
        status, doc = self.post("/gold", {"id": "en-2", "to": ["Jaina"], "copper": 9})
        self.assertEqual((status, doc["sent"], doc["unsure"]), (200, [], ["Jaina"]), doc)
        self.assertEqual(sum("mangos database: exit 1" in line for line in self.wait_logs(4)), 2)
        # Asked again until it answers.
        self.database(CHARACTERS)
        status, doc = self.post("/gold", {"id": "en-3", "to": ["Élise"], "copper": 9})
        self.assertEqual((status, doc["sent"]), (200, ["Élise"]), doc)
        self.assertEqual(len(self.db_calls("mangos")), 3)

    def test_a_console_that_never_answers(self):
        self.console.hold[money("Jaina", 9)] = None
        to = ["Jaina", "Thrall", "Élise"]
        status, doc = self.post("/gold", {"id": "t-2", "to": to, "copper": 9})
        self.assertEqual(
            (status, doc["sent"], doc["unsure"], doc["not_sent"]), (200, [], to[:1], to[1:])
        )
        self.assertEqual(self.console.commands, [money("Jaina", 9)])

    def test_database_down_sends_nothing_and_keeps_no_reply(self):
        self.database(CHARACTERS, down=True)
        status, doc = self.post("/gold", {"id": "db-1", "to": ["Thrall"], "copper": 9})
        self.assertEqual((status, doc["error"]), (500, "internal"))
        self.database(CHARACTERS)
        status, doc = self.post("/gold", {"id": "db-1", "to": ["Thrall"], "copper": 9})
        self.assertEqual((status, doc["sent"]), (200, ["Thrall"]))


# ---- vmangos-console beside the agent ------------------------------------------------------------


class Script(Api):
    """vmangos-console and the agent: one console.lock, one console.pending, one world port (a run
    of the script waits 2 s for the world before its first command, as the agent counts it)."""

    too_long = "vmangos-console: a command is one line of at most 254 bytes\n"
    loading = "vmangos-console: the world is still loading: try again in a minute\n"

    def script(self, *args, stdin=None, **env):
        return subprocess.Popen(
            ["bash", os.path.join(BIN, "vmangos-console"), *args], env=dict(os.environ, **env),
            stdin=stdin, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )

    def utf8(self):
        """LC_ALL for a UTF-8 locale where bash has one (its ${#x} then counts characters)."""
        for name in ("C.UTF-8", "C.utf8", "en_US.UTF-8"):
            r = subprocess.run(
                ["bash", "-c", 'x=é; printf %s "${#x}"'], env=dict(os.environ, LC_ALL=name),
                capture_output=True, text=True, timeout=30,
            )
            if (r.stdout, r.stderr) == ("1", ""):
                return {"LC_ALL": name}
        return {}

    def test_one_line_of_at_most_254_bytes(self):
        """mangosd reads 255 bytes with the newline: the rest of a longer line, or a second line,
        would run inside the next writer's answer. Refused before the lock, nothing written."""
        env = self.utf8()
        with open(os.path.join(self.run_dir, "console.lock"), "w") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            for args in (("x" * 255,), ("announce", "é" * 123), ("announce Hi", "server info\n")):
                started = time.monotonic()
                r = self.script(*args, **env)
                out, err = r.communicate(timeout=30)
                self.assertEqual((r.returncode, out), (1, ""), args)
                self.assertEqual(err, self.too_long)
                self.assertLess(time.monotonic() - started, 5)
        self.assertEqual(self.console.commands, [])
        self.assertEqual(self.read(self.console.log), b"")
        r = self.script("announce", "é" * 122 + "x", **env)
        out, err = r.communicate(timeout=30)
        self.assertEqual((r.returncode, err), (0, ""), out)
        self.assertEqual(self.console.commands, ["announce " + "é" * 122 + "x"])

    def test_commands_from_stdin(self):
        """CRLF lines, blank ones (the last one too) and a last line without its newline are fine;
        a refused line fails the run once the others ran."""
        r = self.script(stdin=subprocess.PIPE)
        out, err = r.communicate("server info\r\n\nserver motd\n\n", timeout=30)
        self.assertEqual((r.returncode, err), (0, ""), out)
        self.assertEqual(out.count("There is no such command."), 2)
        self.assertEqual(self.console.commands, ["server info", "server motd"])
        r = self.script(stdin=subprocess.PIPE)
        out, err = r.communicate("server info\n" + "x" * 255 + "\nserver motd\n", timeout=30)
        self.assertEqual(r.returncode, 1, out)
        self.assertEqual(err, self.too_long)
        self.assertEqual(self.console.commands[2:], ["server info", "server motd"])
        # The last line without its newline, LF or CRLF before it.
        for text in ("server info\nserver motd", "server info\r\nserver motd"):
            r = self.script(stdin=subprocess.PIPE)
            out, err = r.communicate(text, timeout=30)
            self.assertEqual((r.returncode, err), (0, ""), repr(text))
            self.assertEqual(out.count("There is no such command."), 2, repr(text))
        self.assertEqual(self.console.commands[4:], ["server info", "server motd"] * 2)

    def test_the_world_still_loading(self):
        """No world port listening: the script waits its 20 s without the lock, then says so and
        stops, the line after it unsent too, nothing written to the FIFO."""
        self.world(False)
        commands = os.path.join(self.dir, "commands.txt")
        with open(commands, "w") as f:
            f.write("server info\nserver motd\n")
        started = time.monotonic()
        with open(commands) as stdin:
            r = self.script(stdin=stdin)
        time.sleep(1)
        with open(os.path.join(self.run_dir, "console.lock"), "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(lock, fcntl.LOCK_UN)
        out, err = r.communicate(timeout=60)
        self.assertEqual((r.returncode, out, err), (1, "", self.loading))
        self.assertGreater(time.monotonic() - started, 18)
        time.sleep(0.2)
        self.assertEqual(self.console.commands, [])
        self.assertEqual(self.read(self.console.log), b"")

    def test_the_world_comes_up_while_it_waits(self):
        """The port listening from 1 s on: the command goes once it has for 2 s."""
        self.world(False)
        started = time.monotonic()
        r = self.script("server", "info")
        time.sleep(1)
        self.world(True)
        out, err = r.communicate(timeout=30)
        self.assertEqual((r.returncode, err), (0, ""), out)
        self.assertGreater(time.monotonic() - started, 2.9)
        self.assertEqual(self.console.commands, ["server info"])

    def test_the_script_waits_for_the_agents_late_prompt(self):
        # Past the script's 2 s for the world, so the prompt is still due when it takes the lock.
        self.console.hold[money("Jaina", 9)] = 3.0
        status, doc = self.post("/gold", {"id": "s-1", "to": ["Jaina"], "copper": 9})
        self.assertEqual((status, doc["unsure"]), (200, ["Jaina"]))
        self.assertTrue(os.path.exists(self.pending))
        r = self.script("server", "info")
        out, err = r.communicate(timeout=30)
        self.assertEqual((r.returncode, err), (0, ""), out)
        self.assertIn("There is no such command.", out)
        self.assertNotIn("Mail sent to", out)
        self.assertFalse(os.path.exists(self.pending))
        self.assertEqual(self.console.commands, [money("Jaina", 9), "server info"])

    def test_the_agent_waits_for_a_stopped_scripts_prompt(self):
        self.console.hold["server info"] = 1.0
        r = self.script("server", "info")
        for _ in range(100):
            if self.console.commands:
                break
            time.sleep(0.05)
        # Past the write, before the prompt: as Ctrl-C would.
        time.sleep(0.2)
        r.terminate()
        r.communicate(timeout=30)
        self.assertEqual(r.returncode, 143)
        self.assertTrue(os.path.exists(self.pending))
        status, doc = self.post("/gold", {"id": "s-2", "to": ["Thrall"], "copper": 9})
        self.assertEqual((status, doc["sent"]), (200, ["Thrall"]))
        self.assertFalse(os.path.exists(self.pending))


# ---- the commands as the image runs them ---------------------------------------------------------


class Commands(Folders):
    def agent(self, *args, **env):
        return subprocess.Popen(
            [sys.executable, "-I", "-u", AGENT, *args], env=dict(os.environ, **env),
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )

    def test_usage(self):
        r = self.agent()
        r.communicate(timeout=30)
        self.assertEqual(r.returncode, 2)

    def test_serve_refuses_an_unusable_code(self):
        for code in ("", "change-me-admin-0000", "too-short", "VANILLISHADMIN2026"):
            r = self.agent("serve", INILLA_ADMIN_CODE=code)
            out, err = r.communicate(timeout=30)
            self.assertEqual(r.returncode, admin.NO_CODE)
            self.assertEqual(len(err.splitlines()), 1, err)
            self.assertNotIn("change-me", out + err)

    def test_serve(self):
        self.world(False)
        port = free_port()
        r = self.agent("serve", INILLA_ADMIN_PORT=str(port))
        try:
            for _ in range(100):
                try:
                    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
                    conn.request("GET", "/status", headers={"Authorization": f"Bearer {CODE}"})
                    reply = conn.getresponse()
                    doc = json.loads(reply.read())
                    conn.close()
                    break
                except OSError:
                    time.sleep(0.1)
            else:
                self.fail("serve never answered")
            self.assertEqual((reply.status, doc["ok"], doc["world"]), (200, True, "starting"))
            early = self.read_until(r, "GET status 200")
        finally:
            r.terminate()
            out, err = r.communicate(timeout=30)
        self.assertIn(f"listening on port {port}", early + out)
        self.assertIn("GET status 200", early + out)

    def read_until(self, r, text, timeout=5):
        """What the agent has printed by the time it prints text (it logs after the reply)."""
        out, end = b"", time.monotonic() + timeout
        while text.encode() not in out and time.monotonic() < end:
            if select.select([r.stdout], [], [], 0.1)[0]:
                chunk = os.read(r.stdout.fileno(), 4096)
                if not chunk:
                    break
                out += chunk
        return out.decode()


if __name__ == "__main__":
    unittest.main()
