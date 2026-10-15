# Design

How the server is put together, and why. For day-to-day commands, see
[OPERATIONS.md](OPERATIONS.md). It began as iNilla-server, the backend of the iNilla iOS app;
Vanillish adds prebuilt multi-arch images, the one-command setup, the game files in a Docker
volume, `server.json` and the sign-up.

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
| `ghcr.io/klanthier/vanillish-vmangos` | `<the 40-character VMANGOS_COMMIT>-r<N>`: `rN` moves when the stack's scripts or Dockerfile change |
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
| `inilla-mangosd` | render `mangosd.conf`, run mangosd with a scriptable console |

The build follows upstream's own release workflow: Ubuntu 24.04, libmysqlclient, OpenSSL and
zlib, with `BUILD_FOR_HOST_CPU=0` so the image runs on any x86-64 or arm64 CPU. Two flags make the
arm64 build the same program as the x86-64 one: `-fsigned-char` (plain `char` is unsigned on Linux
aarch64, and vmangos' Recast keeps -1 in one, so the movement maps come out wrong without it) and
`-ffp-contract=off` (no fused multiply-adds, so float arithmetic rounds as on x86-64).
`BUILD_JOBS` caps a local compile, whose jobs take up to 1.5 GB each.

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
3. **`config/*.conf`**, the operator's `Key = Value` overrides.

The result lives in `/run/vmangos/` inside the container. It holds the database password, so it is
never written to a volume or the repository.

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
file, writes one command into the FIFO, and prints what arrives up to the next `mangos>` prompt.
That gives account creation and every other console command a plain `docker compose exec`, with no
TTY, no `docker attach`, and no SOAP or RA account to bootstrap first.

## The data host

`inilla-datahost` (Rust, `datahost/`) is the server half of the client's data contract, and its
front door:

| request | answer |
|---|---|
| `GET /` | a one-line hint starting `inilla-datahost`, always open (also the health check) |
| `GET /server.json` | what the server is (below); always open, `Cache-Control: no-store` |
| `POST /accounts` | the sign-up (below) |
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

With `INILLA_DATA_TOKEN` set, everything except `/` and `/server.json` needs
`Authorization: Bearer <token>` (or `?token=`); empty, the host is open, which is the easy way on a
home network. It serves only the archives the client mounts (the eleven required ones plus
`patch-?.MPQ` and `speech2.MPQ`), by name from the manifest, never a path. Repack extras such as
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
  "software": "vanillish-datahost 0.3.0",
  "core": "vmangos",
  "build": 5875,
  "auth": { "port": 3724 },
  "realms": [ { "name": "Vanillish", "kind": "pve",
                "world": { "address": "192.168.1.20", "port": 8085, "same_host": true } } ],
  "data": { "url": "", "token": false, "default": "enUS",
            "locales": [ { "locale": "enUS", "path": "enUS/", "bytes": 5400000000, "addons": 1 } ] },
  "registration": { "open": true, "url": "accounts", "token": false, "credentials": "srp6",
                    "name_min": 3, "name_max": 16, "password_min": 6, "password_max": 16 }
}
```

- A `format` other than 1 makes the app ask for an update; it ignores fields it does not know.
- `auth.host` absent means the host the player typed; `auth` absent means a data host alone.
- `data` absent means no game files here; `registration` absent or `open: false`, no sign-up.
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

## Network and security

- Published: 3724, 8085, 8099, on `BIND_ADDR` (default all interfaces, because the phone is on the
  LAN).
- Not published: MariaDB. Its root and `mangos` passwords come from `.env`, and `db-init` resets the
  `mangos` password to match `.env` on every run.
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
  do not forward 8099 unprotected: it serves Blizzard's files and takes sign-ups. Forwarding 3724
  and 8085 to play from outside also needs `REALM_ADDRESS` to be the public address.
