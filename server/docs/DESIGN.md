# Design

How the server is put together, and why. For day-to-day commands, see
[OPERATIONS.md](OPERATIONS.md). It began as iNilla-server, the backend of the iNilla iOS app;
Vanillish adds prebuilt multi-arch images, the one-command setup, the game files in a Docker
volume, `server.json`, the sign-up and the admin menu.

## What the app needs from a server

Vanillish (and iNilla) is a native 1.12.1 client, benilla on a phone, so it needs exactly what the
original client needed, plus what a phone lacks:

1. **A 1.12.1 core without Warden.** The client speaks the original protocol (SRP6 logon on 3724,
   the world on 8085), reports itself as Windows x86 build 5875 with the matching integrity hash,
   and does not implement Warden. vmangos, which benilla develops against, accepts all of that with
   Warden off.
2. **A reachable world address.** realmd hands out the world server as `address:port` from its
   `realmlist` table. The phone dials that address, so it must be the LAN address of this machine
   and the published port, never `127.0.0.1` or a container name. The Vanillish app goes one step
   further and dials the world at the host the player typed (below), so a stale address does not
   lock it out.
3. **The game data, once.** A desktop client reads `Data/` from disk. The phone gets it from the
   data host over HTTP.
4. **A way in that needs only an address.** `server.json` describes the server, and `POST
   /accounts` makes an account, so the player types an IP and nothing else.

Everything else (TLS, WebSockets, a web front) is the browser clients' business, not this one's.

## The one-command setup

`setup-mac.sh` is the whole setup for macOS, and for Linux and WSL as well (`setup-linux.sh`
hands over to it, so the two cannot drift). It is fetched with
`/bin/bash -c "$(curl -fsSL …)"`, Homebrew's pattern: the script's input stays the terminal, so
it can ask questions, and nothing is downloaded and left quarantined by Gatekeeper.

- **No git, no python, no Homebrew.** On a fresh Mac `/usr/bin/git` and `/usr/bin/python3` are
  stubs that ask to install the developer tools. The script fetches this repository's tarball from
  `codeload.github.com` (about 10 MB) and takes `server/` out of it, with `curl` and `tar`.
- **bash 3.2 and BSD tools**, what macOS ships: no arrays, no `${x,,}`, no `mapfile`, no `sed -i`,
  `stat -c`, `readlink -f` or `timeout`. It is checked with shellcheck, and run under bash 3.2.57
  with bsdtar and stand-ins for the macOS tools (`route`, `ipconfig`, `osascript`, `pmset`, …).
- **Idempotent.** Each step checks for its own result: the passwords are made once, the game files
  copied once, the address refreshed every time. Running it again is how the server is updated, and
  how a new address reaches `.env`. An installed copy hands over to the copy it just downloaded,
  and a running copy of the script is never overwritten in place (bash reads a script as it runs).
- **The questions come first** (the WoW folder, the pathfinding), then the long work, so the
  player can walk away.
- **`docker compose up` runs on its own**: under `nohup`, in a process group of its own, so neither
  a closed window nor Ctrl-C reaches it. (A background job's ignored SIGINT is not enough: the
  docker CLI listens for SIGINT again, and the terminal sends Ctrl-C to its whole foreground
  group.) Compose orders the services on the client side: had `up` been stopped during the map
  extraction, mangosd would never start. The script follows it from the containers' states and
  their last log lines, and says as soon as the data host is up that the phone can start
  downloading. A start that an earlier run left going is followed to its end before the run's own,
  and a map extraction nothing follows any more (an interrupted `up`) is waited for, after which the
  run's own `up` starts the world server.
- **The address.** The default route's interface first (Wi-Fi or Ethernet, whichever macOS
  prefers), then any `en*` with a private IPv4; a VPN's `utun*` route is skipped, and `en0` is
  never assumed (on a Mac mini it is Ethernet). It also prints `<LocalHostName>.local`, which
  Android 12 and later resolve. On WSL it asks Windows for its address, since Docker Desktop
  publishes the ports there.

## Images

Two images, published to GHCR for `linux/amd64` and `linux/arm64`, each built natively (an
`ubuntu-24.04` and an `ubuntu-24.04-arm` runner, no QEMU) and merged into one manifest list:

| image | tag |
|---|---|
| `ghcr.io/klanthier/vanillish-vmangos` | `<the 40-character VMANGOS_COMMIT>-r<N>`: `rN` moves when the stack's scripts, patches or Dockerfile change |
| `ghcr.io/klanthier/vanillish-datahost` | the data host's Cargo version |

Both also carry `sha-<short>` and `latest`. `compose.yaml` keeps `build:` beside `image:` and sets
no `pull_policy`: compose pulls the tag, and builds it here only when the registry does not have
it (a `VMANGOS_COMMIT` of your own, an unpublished fork). Tags are never reused, because compose
does not pull a tag it already has.

`docker/vmangos/Dockerfile` builds vmangos once, and every vmangos service runs that image with a
different command:

| command | role |
|---|---|
| `inilla-db-init` | databases, users (with the sign-up's), the world dump, migrations, the realm row |
| `inilla-extract` | `dbc`/`maps` → `vmaps` → `mmaps` from the client |
| `inilla-realmd` | render `realmd.conf`, run realmd |
| `inilla-mangosd` | render `mangosd.conf` (the admin menu's rates over it), run mangosd with a scriptable console, and the admin menu's agent beside it |

The build follows upstream's own release workflow: Ubuntu 24.04, libmysqlclient, OpenSSL and
zlib, with `BUILD_FOR_HOST_CPU=0` so the image runs on any x86-64 or arm64 CPU. Two flags make the
arm64 build the same program as the x86-64 one: `-fsigned-char` (plain `char` is unsigned on Linux
aarch64, and vmangos' Recast keeps -1 in one, so the movement maps come out wrong without it) and
`-ffp-contract=off` (no fused multiply-adds, so float arithmetic rounds as on x86-64).
`BUILD_JOBS` caps a local compile, whose jobs take up to 1.5 GB each. Before the compile, the
checkout takes Vanillish's patches to vmangos, `docker/vmangos/patches/*.patch` in name order (one:
the admin menu's player speed, [below](#the-patch-ratemovespeedplayer)).

Nothing from a WoW client is ever in a build context: the root `.dockerignore` drops `*.MPQ`,
`*.mpq`, `Data/` and `WTF/` folders, and `.env`, which holds the passwords.

## The game files

The setup copies the player's archives into the external Docker volume `vanillish-wowdata`, and
compose mounts `${WOW_DATA_DIR:-vanillish-wowdata}` read-only at `/client/Data` for the extractor
and the data host. A bind-mounted folder still works (`WOW_DATA_DIR=/path/to/Data`), on Linux and
Windows especially. On a Mac the volume is better:

- **Case.** Docker Desktop's file sharing insists that a file is opened in its original case, and
  the extractors open `dbc.MPQ`, `patch.MPQ` and the rest by name; some clients ship `DBC.MPQ`. In
  the volume they are renamed once, to the stock spelling.
- **It keeps working when the source changes**: the WoW folder moved, its drive unplugged, macOS'
  permission withdrawn.
- **No file sharing to set up.** The copy is one `tar` stream through the Docker API into a
  throwaway container, so it works the same on Docker Desktop, OrbStack and Colima, from any disk.
  On macOS, `COPYFILE_DISABLE=1` and bsdtar's `--no-mac-metadata --no-xattrs` keep AppleDouble
  `._` files and quarantine attributes out of it.
- **Speed.** The extractors' random reads and the data host's hashing stay on the VM's own disk,
  not on VirtioFS.
- **`docker compose down -v` keeps it**, as an external volume: a fresh realm does not mean
  copying 5 GB again.

Only the archives the client mounts travel (the 11 required ones, `patch-?.MPQ`, `speech2.MPQ`);
the new ones land beside the old ones and replace them only once the copy is whole, and the
setup compares every archive's size in the volume with the source.

## Startup order

```
db (MariaDB, healthy) ──► db-init (one-shot) ──► realmd
                                          ├────► mangosd ◄── extract (one-shot)
                                          └────► datahost (no game data needed)
```

`db-init` and `extract` run on every `up` and exit 0. `realmd`, `mangosd` and `datahost` wait for
them with `service_completed_successfully`; the data host only for `db-init` (its sign-up user), so
`server.json`, the sign-up and the game files' download are there while the maps are extracted. Both one-shots check for their own results first, so after the
first run they take a second. A failure stops the start with the reason in the job's log.

When the player asked for pathfinding (`EXTRACT_MMAPS=1`) and there is none yet, the setup's first
`up` runs with `EXTRACT_MMAPS=0`, so the world opens in minutes. It then starts `extract` alone
with the setting on, and the hours of generation run in the background, as the extract service's
own container: a later `up` finds it running and leaves it alone (and waits for it). mangosd turns
mmaps on only when it starts with them on disk, so the setup restarts it once they are there.

## Configuration

The configs are regenerated from `/opt/vmangos/etc/*.conf.dist` on every start, in three layers:

1. **vmangos' defaults**, so new keys in a newer core arrive by themselves;
2. **what the stack owns**: database strings, paths, ports, `Console.Enable`, `GameType` from
   `REALM_TYPE`, Warden off, and vmap/mmap switches set by whether that data exists (vmangos refuses
   to start with mmaps on and none on disk, and runs without line of sight when vmaps are missing);
3. **`config/*.conf`**, the operator's `Key = Value` overrides;
4. for mangosd, **the admin menu's rates** (the `admin` volume), when `ADMIN_CODE` is set
   ([below](#the-agent)).

The result lives in `/run/vmangos/` inside the container, with mangosd's first three layers kept
as `mangosd.base.conf` (what the menu's **Server defaults** go back to). It holds the database
password, so it is never written to a volume or the repository.

## Database

vmangos publishes its world database only as a release asset (`db_latest`, a dump named after the
commit it was taken at; an older asset disappears when a new one is published). The image carries
the dump of its build. `db-init` imports it into databases that are still empty, then applies the
core's `sql/migrations` that the database's `migrations` table does not list yet. Every vmangos
migration registers itself there, so this is exactly how vmangos expects an upgrade to go, and an
existing realm moves forward with each new `VMANGOS_COMMIT` without re-importing. If the dump is
newer than the core, `db-init` counts the migrations it does not know and warns.

MariaDB runs with `sql_mode=NO_ENGINE_SUBSTITUTION`. vmangos is MaNGOS-era code and expects the
permissive mode: zero dates, and silent truncation.

## Server data

`extract` writes into the `serverdata` volume, in the layout mangosd expects:

```
/vmangos/data/5875/dbc/    DBCs, namespaced by client build (mangosd reads DataDir/<build>/dbc/)
/vmangos/data/maps/        terrain and liquids (MapExtractor, high-resolution heights)
/vmangos/data/vmaps/       line of sight and indoor checks (VMapExtractor + VMapAssembler)
/vmangos/data/mmaps/       navmeshes (MoveMapGenerator via mmap_extract.py), optional
/vmangos/data/.inilla-*    one marker per finished step
```

On a 12-core x86-64 machine, measured: `dbc` + `maps` take about 1 minute, `vmaps` about 2
minutes; both are single-threaded, and an M4's cores are at least as fast. mmaps take hours (one
process per map, so the two continents bound the run), hence opt-in and in the background.
mangosd mounts the volume read-only.

## The console

mangosd reads commands from stdin. When stdin closes, it shuts the world down cleanly, which in a
container looks like a crash loop with exit code 0. `inilla-mangosd` gives it a FIFO that the
process itself holds open read-write, so stdin never reaches EOF and a writer never blocks. mangosd's
output also goes through `tee` into `/run/vmangos/console.log`. `vmangos-console` truncates that
file, writes one command into the FIFO, and prints what arrives up to the next `mangos>` prompt
(given none, it takes them from stdin, one per line, skips a blank line, and exits non-zero only
when a command failed). That gives account creation and every other console command a plain
`docker compose exec`, with no TTY, no `docker attach`, and no SOAP or RA account to bootstrap
first.

The admin menu's agent drives the same console, the same way, so the two take turns: each command
holds `/run/vmangos/console.lock` (`flock`) from the truncation to the prompt, or one would read the
other's answer. Both first need the world up: its port, 8085, listening for 2 s (`/proc/net/tcp` or
`tcp6`, the health check's pattern). mangosd opens it once the world has loaded, just after the
console printed its first `mangos>`, which is no command's answer. `vmangos-console` waits for that
within a command's 20 s, outside the lock (the first command of a run; a later one checks that the
port still listens), else exits 1 with nothing written ("the world is still loading"), the commands
after it unsent too. It waits what is left of the 20 s for the lock ("the console stayed busy for
20 s"), then 15 s for the prompt; the agent answers 503 `world_starting` at once, waits 5 s for the
lock (then 503 `console_busy`) and 15 s for the prompt. A command that timed out (or a
`vmangos-console` stopped before its prompt) still runs later and prints its own prompt, after the
lock is gone. So its writer, either of the two, leaves `/run/vmangos/console.pending`, the offset in
`console.log` past which that prompt will come, and whoever takes the lock next waits for that
prompt in the log as it is before truncating it and clears the file: the agent up to 5 s (then 503
`console_busy`), `vmangos-console` for what is left of its 20 s ("an earlier command is still
running"). Only a writer that timed out or was interrupted leaves the file: nothing guesses from an
answer's text that a prompt is still due. A line is at most 254 bytes: mangosd reads 255 with the
newline, and the rest of a longer one would run as a second command, so both writers refuse a longer
command, or one with a line break.

## The data host

`inilla-datahost` (Rust, `datahost/`) is the server half of the client's data contract, and its
front door:

| request | answer |
|---|---|
| `GET /` | a one-line hint starting `inilla-datahost`, always open (also the health check) |
| `GET /server.json` | what the server is (below); always open, `Cache-Control: no-store` |
| `POST /accounts` | the sign-up (below) |
| `GET /admin/nonce` | a single-use nonce for the admin menu ([below](#the-door-a-signature-over-a-nonce)); JSON, like every `/admin/*` answer |
| `GET /admin/status`, `POST /admin/rates`, `POST /admin/rates/reset`, `POST /admin/gold` | the admin menu, signed with the admin code, checked here and passed on to the world server's agent |
| `GET /locales.json` | `{"default":"enUS","locales":[{"locale","path":"deDE/","bytes","addons"}]}` |
| `GET [/<locale>]/manifest.json` | `{"format":1,"build":5875,"locale":"enUS","files":[{"name","size","sha256"}]}` |
| `GET\|HEAD [/<locale>]/files/<name>` | the archive. `Range: bytes=a-b` → `206`, so an interrupted download resumes |
| `GET [/<locale>]/addons.json` | `{"format":1,"locale","addons":[{"name","version","files":[{"path","size","sha256"}]}]}` |
| `GET\|HEAD [/<locale>]/addons/<path>` | one addon file |

Without a `/<locale>` prefix a request means the first `--data`, so an app that only knows
`/manifest.json` still works. A `--data` folder with no archives is skipped, which is how the
optional German one stays mounted empty, and how the host starts before any game files are there
(`server.json` and the sign-up work from the first second). Addons come from `--addons` folders,
for every language or one: the image's pfQuest builds, then `addons/all`, `addons/<locale>`; a
later folder's addon replaces an earlier one of the same name.

With `INILLA_DATA_TOKEN` set, everything except `/`, `/server.json` and `/admin/*` (signed with
the admin code instead) needs `Authorization: Bearer <token>` (or `?token=`); empty, the host is
open, which is the easy way on a home network. It serves only the archives the client mounts (the
eleven required ones plus `patch-?.MPQ` and `speech2.MPQ`), by name from the manifest, never a
path. Repack extras such as
`base.MPQ` and `backup.MPQ` are left out, because the client never opens them. The client refuses
a manifest whose `format` or `build` it does not know, or one that lacks a required archive, so a
change to this shape must land in the app too. `datahost/src/manifest.rs` and the client's `data`
module have to agree.

Hashing 5.4 GB takes about 30 seconds. The result is cached in the `datahost` volume
(`.inilla-manifest-<locale>.json`), keyed by every archive's name, size and mtime; the copy into
the game files' volume keeps the archives' mtimes.

### `server.json`

```json
{
  "format": 1,
  "name": "Vanillish",
  "software": "vanillish-datahost 0.4.0",
  "core": "vmangos",
  "build": 5875,
  "auth": { "port": 3724 },
  "realms": [ { "name": "Vanillish", "kind": "pve",
                "world": { "address": "192.168.1.20", "port": 8085, "same_host": true } } ],
  "data": { "url": "", "token": false, "default": "enUS",
            "locales": [ { "locale": "enUS", "path": "enUS/", "bytes": 5400000000, "addons": 1 } ] },
  "registration": { "open": true, "url": "accounts", "token": false, "credentials": "srp6",
                    "name_min": 3, "name_max": 16, "password_min": 6, "password_max": 16 },
  "admin": { "url": "admin/", "open": true }
}
```

- A `format` other than 1 makes the app ask for an update; it ignores fields it does not know.
- `auth.host` absent means the host the player typed; `auth` absent means a data host alone.
- `data` absent means no game files here; `registration` absent or `open: false`, no sign-up;
  `admin` absent, no admin menu, and `open: false`, it does not answer this requester (`lan`, a
  public address).
- Relative `url`s resolve against where `server.json` came from (`""`: the same place).
- `same_host: true` says the world runs where the logon does. When every realm is on the same
  host and realmd hands out another address, the app dials the typed host instead: a Mac whose
  DHCP lease changed, or a container that cannot know the Mac's LAN address, keeps working.

In the full stack it comes from `.env`: the realm's name, type and address, the ports, the token
and the sign-up setting (the data host's `--server-name`, `--realm-name`, `--world-address`, …
flags, or their `INILLA_*` variables).

### The sign-up

1.12 has no sign-up, and the obvious ways to add one need the world loaded (SOAP, the console) or
hand the password to the server. Instead:

- **The phone computes the account's SRP6 salt and verifier itself** (`benilla_srp`) and posts
  `{"username","salt","verifier"}` as JSON. The password never leaves the phone.
- **The data host inserts it with a database user that can do nothing else.** `db-init` creates
  `vanillish_signup` with `SELECT (id, username, s)` and `INSERT (username, v, s)` on
  `realmd.account`, `SELECT (id)` on `realmd.realmlist` and `INSERT` on `realmd.realmcharacters`.
  It cannot read a verifier, change an account, or touch `account_access`, so no sign-up is ever a
  game master. The name's UNIQUE key refuses a taken name (409); the same request replayed after a
  lost answer finds its own salt and gets 200.
- **It needs only `db` and `db-init`**: no loaded world and no extracted maps. A player can make an
  account during the first start, while the game files download.
- **Guards.** `INILLA_SIGNUP=lan` (compose.yaml's default) takes requests from private addresses
  only; `off` is the binary's default and `compose.datahost.yaml`'s. The token, when set, is
  required too. `Content-Type: application/json` is required, which a web page cannot send across
  origins without a preflight the host never answers. Rate limits count the attempts that reach the
  database: 5 in 10 minutes and 20 a day per address, 30 an hour in all, then 429 with
  `Retry-After`.

Every reply is `{"ok", "username"?, "error"?, "message", "retry_after"?}`, with the status saying
what happened (201 created, 200 replay, 400, 401 token, 403 closed, 409 taken, 413, 415, 429, 503
database not ready, 500).

## The admin menu

The owner's phone changes the realm's rates and gives gold through two hops:

```
app (Settings → Server)
  │ GET  server.json           "admin": {"url": "admin/", "open": true}
  │ GET  admin/nonce           a single-use nonce
  │ GET|POST admin/<route>     Authorization: Vanillish-Admin <nonce>:<mac>
  ▼
datahost :8099   checks the signature; forwards with Authorization: Bearer <ADMIN_CODE>
  ▼
vanillish-admin serve, in the mangosd container, :8091 on compose's network only
  ├─ rates   /vmangos/admin/rates.conf (the admin volume) → /run/vmangos/mangosd.conf → `reload config`
  ├─ gold    the names, from the characters database → `send money`, one per character
  └─ status  each rate live and in the server's own settings, who is online, the world up or not
```

The menu is on when `ADMIN_CODE` is usable, `INILLA_ADMIN` is not `off`, and the data host has an
agent to forward to (`INILLA_ADMIN_AGENT`: `mangosd:8091` in `compose.yaml`; `compose.datahost.yaml`
names none, so there it is off). Off, every admin route answers 404 `admin_off`, and `server.json`
has no `admin`. The data host's startup line says whether it is on, or why not, never with the
code.

### The code

The app, the data host, the agent and the setup read `ADMIN_CODE` the same way: ASCII whitespace
(Rust's `is_ascii_whitespace`: space, tab, LF, FF and CR, so a vertical tab is refused) and dashes
dropped, ASCII letters upper-cased (an `ß` must not become `SS`), any other character refused. It
is usable only when it is random: 16 or more of the setup's 32 letters, or 20 or more hex digits
(`openssl rand -hex 10`, also 80 bits), and not starting with `CHANGEME` (the example's
placeholder). A code picked by hand leaves the menu off: the wrong-code limit slows no guesser
([below](#the-door-a-signature-over-a-nonce)), so the code's 80 bits are the guard. The setup
makes 16 of 32 letters (no I, O, 0 or 1), each the low 5 bits of a byte from `/dev/urandom`
(uniform: 256 = 8 × 32), printed as `ABCD-EFGH-JKLM-NPQR`. It makes one only when `.env` has no
`ADMIN_CODE` line, or still a change-me, read as the data host reads it (`CHANGE-ME` and
`changeme` too): an empty value is the owner's off switch, and stays. It reads the line as compose
hands it to the containers: blanks around it dropped, a quoted value up to its closing quote, an
unquoted one cut at a ` #` comment (a space, then `#`; a `#` after anything else is part of the
value). A code it keeps that the
data host would refuse gets a warning, and the summary says why the menu is off instead of where
the code is. The summary prints the code only in the run that made it, never in a dry run, whose
`.env` listing hides it as it hides the passwords; made on a run that started nothing (the
pathfinding still running), it says that the code takes effect after the next run.

Only what uses the code holds it. The data host takes it from its environment, never a flag (a
flag shows in `ps`). In the mangosd container it is for the agent: `inilla-mangosd` starts the
agent, then unsets it before `tee` and mangosd start. realmd, db-init and extract never get it
(`ci/check.sh` checks the resolved compose file).

### The door: a signature over a nonce

The data host is plain HTTP on a home network, where a password sent with each request would cross
the Wi-Fi in clear at every tap. So the code never travels:

- `GET /admin/nonce` (no auth) → `{"ok":true,"nonce":"<32 hex>","expires_in":60}`, good for 60 s
  and one request. A nonce proves itself: 6 random bytes, the second it was made and a 6-byte tag,
  HMAC-SHA256 under a secret this host draws from `/dev/urandom` at start. Handing one out stores
  nothing, so no one asking for nonces pushes the owner's out; a nonce is kept, as spent, only
  once a request signed with the code used it, for its minute.
- The request carries `Authorization: Vanillish-Admin <nonce>:<mac>`, with
  `key = SHA-256(normalised code)` and
  `mac = HMAC-SHA256(key, nonce "\n" METHOD "\n" route "\n" hex(SHA-256(body)))`, lowercase hex.
  `route` is the data host's own path (`/admin/status`), whatever prefix a proxy puts in front of
  it; the body is its raw bytes, empty for a GET. Only the header signs: a `?token=` is ignored.
- A captured request cannot be sent again (its nonce is spent) or changed (the MAC covers the
  method, the route and the body). What it asks can still be read, as anything on plain HTTP can:
  HTTPS in front (a reverse proxy) or a VPN hides that too.

| route | method | forwarded to the agent's |
|---|---|---|
| `/admin/nonce` | GET | — (answered here) |
| `/admin/status` | GET | `GET /status` |
| `/admin/rates` | POST | `POST /rates` |
| `/admin/rates/reset` | POST | `POST /rates/reset` |
| `/admin/gold` | POST | `POST /gold` |

A request is refused in this order, each answer `{"ok":false,"error","message","retry_after"?}`:
a route not in the table, 404 `not_found`; the menu off, 404 `admin_off`; under `lan` a peer that
is not private, 403 `admin_closed`; another method, 405 with `Allow`; no readable header, 401
`admin_auth`; a nonce this host did not make (its tag) or older than 60 s, 401 `admin_nonce`; a
body over 4 KiB, 413 `too_large` (a body left unread cannot be checked against the signature); a
wrong MAC, 401 `admin_code`, or 429 `too_many`; a nonce already spent, 401 `admin_nonce` (only a
right MAC spends one); then, for a POST, a content type other than `application/json`, 415
`json_only`, and no body, 400 `bad_request`. Only then is it forwarded.

**The limiter counts wrong codes only, never stops the right one, and is advisory.** Each wrong
MAC counts against its source (an IPv4 address, an IPv6 /64); a source with 10 in the last 10
minutes gets 429 `too_many` for its wrong ones, with `Retry-After`, until the oldest leaves the
window. A right MAC is served whatever the count: behind Docker Desktop, where every phone shares
the gateway's address, a limiter that stopped the owner too would let anyone on the network lock
the owner out. So the 429 tells an honest app its code is wrong (the app forgets the saved code
on the data host's own `too_many`, as on `admin_code`, and asks for it again); it slows no guesser
who ignores it, and what guards the menu is the code's 80 bits, which is why a hand-picked code is
refused ([The code](#the-code)): over plain HTTP, one request seen on the network would also let a
weak code be tested offline. A missing header or a stale nonce is not counted: neither says anything
about the code, and neither reaches the agent.

`application/json` keeps a web page from posting here behind the owner's back, as for the sign-up:
a browser has to ask first (CORS), and this host never says yes. A request's log line is `<peer>
admin <route name> <status>`, the name from the table (`unknown` otherwise): never the path, the
header, the code or a nonce.

**Forwarding**: one TCP connection per request, `Host: mangosd`, `Authorization: Bearer <ADMIN_CODE
normalised>` (the agent compares normalised codes, and nothing but letters and digits reaches the
header), `Content-Type: application/json`, `Connection: close`, the body verbatim; 3 s to connect,
25 s for the whole answer, at most 64 KiB. No connection is 503 `world_unavailable` ("The world
server is not running (or still starting)."); no whole answer, by the deadline or because the
connection ended after the request went (the agent dies with mangosd), 504 `no_answer` ("it may
still have been done"); the agent's 401 or 403, or an answer that is not HTTP, over 64 KiB or not
JSON, 502 `admin_misconfigured` (the two disagree on `ADMIN_CODE`: `docker compose up -d` gives
both the one in `.env`). Any other answer passes through, status and body as the agent gave them.

`server.json` says `"admin": {"url": "admin/", "open": <bool>}` only while the menu is on; `open`
is false for a public requester under `lan` (both documents are made once at start, as for the
sign-up's `registration`).

### Why an agent in mangosd, and not RA or SOAP

- **The rates have to be written in mangosd's container.** `reload config` re-reads the file
  mangosd was started with (`-c /run/vmangos/mangosd.conf`), which that container makes at every
  start, and a rate outlives a restart only if it is replayed into it then. The console's FIFO is
  there too. So something has to run in that container, and the smallest such thing is an agent
  there that the data host talks to over compose's network.
- **RA and SOAP** are compiled in and off, their switches read only at start, and each needs an
  account with an administrator's GM level whose password then lives somewhere and crosses the
  network in clear (RA is telnet, SOAP HTTP basic auth). Over both, a command that is not available still
  reports success. The console the stack already drives needs no account.
- **The data host keeps its rights**: still the only public port of the two, with no console and
  no new database user (its only one is the sign-up's). The agent checks everything again (its own
  allowlist of keys, the ranges, the names) and sends only fixed console commands: `reload config`,
  an `announce` made of numbers, and `send money` to a name as the database spells it.
- **The agent's port, 8091, is never published**, and it checks the bearer code (in constant time)
  before it reads a body. Any container on compose's network can reach it, which grants nothing
  new: realmd and mangosd already hold the database password, with every right.

### The agent

`vanillish-admin` (Python, standard library only, in the vmangos image) has two commands.

**`render`**, run by `inilla-mangosd` before mangosd starts when `INILLA_ADMIN_CODE` is set:
`/run/vmangos/mangosd.conf` is `mangosd.base.conf` with the valid lines of
`/vmangos/admin/rates.conf` written in. A line counts when its key is one of the menu's, in vmangos'
exact case (its lookup is case-sensitive), and its value plain digits (`2`, `1.5`) within the
range below; any other line is dropped with a log line, since mangosd reads a value with
`std::stof`, which throws, and so stops the world, on text. A key the base has is rewritten in
place on its first uncommented line (vmangos keeps a key's first occurrence, so an appended
duplicate would change nothing), a key it lacks is appended, and every other line passes through
byte for byte. A render that fails starts the world on the base. With no code, the overrides stay
in the volume, unapplied, and the log counts them.

**`serve`**, kept running beside mangosd on 8091 (restarted 5 s after it stops, unless the code
is unusable): a thread per request, at most 8 at once (else 503 `busy`), 10 s socket timeouts, a
20 s deadline per request (under the data host's 25 s and the app's 30 s), bodies up to 64 KiB,
strict JSON (no `NaN`, no booleans for numbers). One lock covers every change and every console
use (5 s to get it, then 503 `console_busy`); the console is shared with `vmangos-console`
([The console](#the-console)). A change needs the world up: mangosd's world port listening for 2 s
(`/proc/net/tcp`, the health check's pattern), else 503 `world_starting`.

| key | the API's range |
|---|---|
| `Rate.XP.Kill`, `Rate.XP.Quest`, `Rate.XP.Explore` | 0 to 20 |
| `Rate.Drop.Item.Poor`, `.Normal`, `.Uncommon`, `.Rare`, `.Epic`, `.Legendary`, `.Referenced` | 0 to 20 |
| `Rate.Drop.Money` | 0 to 20 |
| `Rate.MoveSpeed.Player`, when the image lists the patch `0001-move-speed` | 0.5 to 3 |

The app's sliders go from ×0.5 to ×10 (the speed ×0.75 to ×2), widened to take in a value set
otherwise. What the knobs reach is vmangos': an item row drops at its chance times the rate of its
quality, a reference row (a shared loot table) is rolled at its chance times `Referenced`, and quest
items, white mostly, follow their quality. Rows in a loot group, the one-of-a-list drops of bosses
and dungeons, are rolled with no rate at all. `Rate.Drop.Money` multiplies the money in loot and in
quest rewards.

- `GET /status`: each rate's `value` (what the live conf holds), `server` (the base's, read as
  vmangos reads it; 1 when absent) and range; `overrides`, how many rates `rates.conf` holds, one
  kept for a knob this image lacks included (the app offers **Server defaults** while it is not
  0); the characters online (by name, at most 200); `world`, `up` or `starting`.
- `POST /rates {"id","rates":{key:number},"announce":bool}`: checked all or nothing, each value
  rounded to 0.01; a value equal to the server's own drops its override. Nothing moved: no reload.
  Else `rates.conf` is written (a temporary file, renamed), the conf rendered, `reload config` run,
  and with `announce` the players read `Server rates: XP x2, loot x1.5, gold x1, speed x1.2` in chat.
  `reload config` re-reads every setting, so the message of the day and the player limit go back to
  the file's as well (a `.server set motd` is lost). A rate for a knob this image lacks (the speed,
  without its patch) stays in `rates.conf`, unapplied, through every other change, for an image
  that has it. `POST /rates/reset {"id","announce"}` deletes `rates.conf`, such a line too (alone
  in it as well), and does the same.
- `POST /gold {"id","to":["Name",…] | "online","copper":n}`: 1 copper to 100 000 gold, 1 to 50 names
  of 2 to 12 letters. One query on the characters database finds them, as the stack's `mangos` user
  (its password in the child's `MYSQL_PWD` only), the names hex-encoded into the SQL and deleted
  characters left out; the database's spelling is checked again (a name counts in any case, by
  `str.lower`: mangosd's name maps are exact once it has spelled a name with its first letter upper
  and the rest lower, so Weiß and Weiss are two characters), then one
  `send money <Name> "A gift from the realm" "Sent with the server's admin menu." <copper>` each, a
  mail whether they are online or not. The answer sorts the names into `sent` (the console answered
  with the realm's own "mail sent" sentence, `mangos_string` 169 in any of its languages, so a
  German realm works too), `missing`, `unsure` (any other answer, or no prompt in time: a mailbox
  says) and `not_sent` (the deadline came first, or the console stayed busy). The app offers to send
  to the `not_sent` alone, in sends of at most 50, each under an id of its own.
- **Request ids.** A POST may carry an `id`, and a gold one must: the agent keeps the replies to
  the last 32, in memory, and answers a repeated id with its stored reply, doing nothing again. A
  send that got no answer goes again from the app under the same id, so the gold is not paid twice
  while the agent runs; a restart of mangosd forgets the ids, which is why the app says to check a
  mailbox first. A 5xx that is not JSON on a POST (a reverse proxy's error page) counts in the app
  as no answer: the request may have reached the agent, and been done.

### The patch: `Rate.MoveSpeed.Player`

vmangos has no server-wide player speed, and a GM's `.modify speed` is lost at the next speed aura,
mount or logout. `docker/vmangos/patches/0001-move-speed.patch` (GPL-2.0-or-later, like the code it
changes) adds a reloadable setting: 1 by default, kept within 0.1 to 10, NaN read as 1.
`Unit::UpdateSpeed` multiplies a player's walk, run, run-back and swim speed by it, mounts and
speed auras included, ahead of the normal-speed cap, so an aura that sets a fixed speed still does;
backward swimming, which vmangos never recomputes, and taxi flights keep their speed. A `reload
config` that changes it hands it to every player at once (one between two maps without a packet:
the new map's create packet carries it), and a login applies it before the first step.

The Dockerfile applies `docker/vmangos/patches/*.patch` after the checkout, in name order.
**Lenient** by default: a patch that does not apply (another `VMANGOS_COMMIT`) is left out with a
warning, and the server is built without what it adds. **Strict**, `VANILLISH_PATCHES=strict`, what
`server-check` and `server-images` build, fails instead. The names applied go in
`/opt/vmangos/vanillish-patches`, where the agent looks before it offers the speed, and the patches
in `/opt/vmangos/patches`, beside vmangos' `LICENSE`. `.gitattributes` keeps the patch files byte
for byte (vmangos' `Unit.cpp` has CRLF endings).

## Network and security

- Published: 3724, 8085, 8099, on `BIND_ADDR` (default all interfaces, because the phone is on the
  LAN).
- Not published: MariaDB, and the admin menu's agent (8091, in the mangosd container). MariaDB's
  root and `mangos` passwords come from `.env`, and `db-init` resets the `mangos` password to match
  `.env` on every run.
- **On a Mac**, Docker Desktop and OrbStack both publish ports on every interface, so the phone
  reaches them; Colima does too. macOS 15's Local Network privacy does not apply: accepting
  incoming connections needs no permission, and tools run from Terminal are exempt for their own
  outgoing ones (Apple's TN3179). The macOS firewall, when on, lets the signed Docker apps through
  unless *Block all incoming connections* is set.
- The container sees every phone as coming from the runtime's gateway, not from the phone. realmd's
  `localAddress` is therefore set to the same address as `address`, since the subnet test that
  picks between the two would mean nothing; and the sign-up's per-address limit acts for everyone
  at once there.
- The data host is plain HTTP, open by default, which is fine on a home LAN. For the internet, put
  it behind HTTPS (the app speaks both) or reach it over a VPN such as Tailscale or WireGuard, and
  do not forward 8099 unprotected: it serves Blizzard's files, takes sign-ups and the admin menu's
  requests (which still need the code). Forwarding 3724 and 8085 to play from outside also needs
  `REALM_ADDRESS` to be the public address.
