<div align="center">
  <h1>Vanillish — server</h1>
  <p><b>Your own World of Warcraft 1.12.1 realm for the <a href="../README.md">Vanillish</a> Android app: on a Mac at home (or any PC), in one command</b></p>
</div>

The phone needs two things from a computer on your Wi-Fi: a realm to play on, and the game's
files, once. This folder is both. It runs [vmangos](https://github.com/vmangos/core), a 1.12.1
server, and a small **data host** that hands the app *your own* game files and lets it make
accounts. In the app, you type this computer's address and nothing else.

## On a Mac with Apple silicon (M1 to M4): one command

1. **Install Docker Desktop or OrbStack**, and open it once.
   - [OrbStack](https://orbstack.dev/download) is the lighter of the two for a Mac that serves all
     day (free for personal use): `brew install --cask orbstack`.
   - [Docker Desktop](https://www.docker.com/products/docker-desktop/) (free for personal use and
     small businesses): `brew install --cask docker-desktop`.
2. **Open Terminal and paste:**

   ```sh
   /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/klanthier/vanillish/klanthier/inilla-android-port/server/setup-mac.sh)"
   ```

   It asks for your World of Warcraft 1.12.1 folder in a Finder window, copies the game's
   archives into Docker (about a minute), downloads the prebuilt server (native arm64: nothing is
   compiled), starts it, and follows it until the realm is up. Count 5 to 10 minutes the first
   time.
3. **It ends with the address to type:**

   ```
   ╔════════════════════════════════════════════════════════════╗
   ║   Your Vanillish server is running.                        ║
   ║   On your phone, open Vanillish and type:                  ║
   ║         192.168.1.20                                       ║
   ║         (or Kevins-Mac-mini.local)                         ║
   ╚════════════════════════════════════════════════════════════╝
   ```

4. **On the phone**, on the same Wi-Fi: open Vanillish, type that address under **Server address**,
   and tap **Connect**. The app downloads the game files from your Mac (5.4 GB, once), you tap
   **Create account**, and you play. The game files can download while the Mac is still loading
   the world: the setup says when the phone can start.

Run the same command again to update the server, or when the Mac has a new address. It keeps
your passwords, accounts, characters and game files, and skips every step that is already done.
The server lives in `~/Vanillish-server`, and `~/Vanillish-server/setup-mac.sh` is the same
script, for next time.

**What it asks, and what it checks:**

| step | what happens |
|---|---|
| the server's files | downloaded from this repository into `~/Vanillish-server`; your `.env`, `config/` and `addons/` survive an update |
| this Mac | Apple silicon, the macOS version |
| Docker | Docker Desktop, OrbStack or Colima, started for you if it is not running; a warning if it has less than 4 GB of memory |
| the address | the Mac's address on your network (the Wi-Fi or Ethernet one, never a VPN's), and its `.local` name |
| `.env` | new database passwords, made once with `openssl`; the address refreshed on every run |
| your game files | a Finder window to pick the WoW folder (or `--data "/path/to/World of Warcraft"`). It needs the 11 archives of a 1.12.1 client (`dbc`, `fonts`, `interface`, `misc`, `model`, `patch`, `sound`, `speech`, `terrain`, `texture`, `wmo`); an English (enUS) client is the tested one. Only the archives the game opens are copied, into the Docker volume `vanillish-wowdata`, and every one's size is checked against the original. |
| creature pathfinding | optional, asked once: monsters walk around obstacles instead of through them. Generated in the background after the first start, about 1.5 to 3 hours on an M4; you can play meanwhile. |
| start | the prebuilt images, then the first start, with its progress: the database (a minute), the server's maps from your game files (3 to 5 minutes), the world (1 to 2 minutes) |

Closing the Terminal window during the first start does not stop it: the server finishes by
itself. `setup-mac.sh --help` lists the options (`--data`, `--ip`, `--reimport`, `--mmaps`,
`--yes`, `--dry-run`, `--uninstall`).

### Keep it reachable

- **The Mac must stay awake while you play.** A sleeping Mac's server cannot be reached. System
  Settings → Energy → *Prevent automatic sleeping when the display is off* (a laptop: Battery →
  Options), or leave `caffeinate -i` running in a Terminal window while you play.
- **Docker must start when you log in**, or the server stays off after a restart: Docker Desktop →
  Settings → General → *Start Docker Desktop when you sign in to your computer*; OrbStack → Settings
  → *Start at login*. With that, the server starts by itself.
- **The firewall.** If macOS asks whether Docker (or `com.docker.backend`) may accept incoming
  connections, answer **Allow**. *Block all incoming connections* must stay off.
- **A laptop** on another Wi-Fi (a café, a hotel) would offer your game files to everyone on that
  network: `cd ~/Vanillish-server && docker compose stop` away from home, `docker compose start`
  back home.

The setup checks the sleep setting, Docker Desktop's start at login and the firewall, and names
whichever needs a change.

## On Linux, or on Windows (WSL)

```sh
bash -c "$(curl -fsSL https://raw.githubusercontent.com/klanthier/vanillish/klanthier/inilla-android-port/server/setup-linux.sh)"
```

The same setup, with the same steps and options; it lives in `~/Vanillish-server` too.

- **Linux**: Docker Engine with its compose plugin, and a user in the `docker` group. The folder
  question is a prompt: type or paste the path. Images exist for x86-64 and arm64.
- **Windows**: Docker Desktop with WSL 2, its *WSL integration* turned on for your distribution
  (Ubuntu, say), and the command run in that distribution's terminal. A Windows path works at the
  prompt (`C:\Games\World of Warcraft`). The phone connects to Windows' own address, which the
  setup asks Windows for. Windows' firewall must let *Docker Desktop Backend* accept connections
  on private networks.

## By hand

The setup only automates these steps; every one of them can be done by hand, on any machine:

```sh
cp .env.example .env     # set REALM_ADDRESS, and the three passwords (openssl rand -hex 16)
```

- **The game files**: either set `WOW_DATA_DIR` in `.env` to your client's `Data` folder (a
  bind mount: no copy, fine on Linux and Windows), or copy the archives into the volume once:

  ```sh
  docker volume create vanillish-wowdata
  tar -cf - -C "/path/to/World of Warcraft/Data" dbc.MPQ fonts.MPQ interface.MPQ misc.MPQ \
      model.MPQ patch.MPQ patch-2.MPQ sound.MPQ speech.MPQ terrain.MPQ texture.MPQ wmo.MPQ |
    docker run --rm -i -v vanillish-wowdata:/d alpine:3 tar -xf - -C /d
  ```

  In the volume the archives must carry these stock names, in this case (the extractors open them
  by name, and Linux is case-sensitive); add `speech2.MPQ` if your client has one.
- **Start**: `docker compose up -d`. Compose downloads the prebuilt images; it builds one here only
  when the registry does not have it (your own `VMANGOS_COMMIT`, or an unpublished fork), which
  takes about 10 minutes more.

## What runs

| service | port | what the app uses it for |
|---|---|---|
| `realmd` | 3724 | the logon |
| `mangosd` | 8085 | the world |
| `datahost` | 8099 | `server.json` (what the app reads after you type the address), the one-time download of the game files (resumable, SHA-256 checked; pfQuest comes with them), and the sign-up |
| `db` | — | MariaDB for vmangos. Never published. |
| `db-init`, `extract` | — | one-shot jobs on every start: the database (and its upgrades), and the server's maps, extracted once from your game files. They exit when done. |

The server is vmangos, built from source at a pinned commit, with Warden off (the app does not
implement it). The images are published as `ghcr.io/klanthier/vanillish-vmangos` and
`ghcr.io/klanthier/vanillish-datahost`, for linux/amd64 and linux/arm64, each built natively. **No
image contains anything from Blizzard**: your game files stay in a Docker volume on your own
machine.

## The app's quick connect

After **Connect**, the app reads `http://<address>:8099/server.json`. It names the realm, where
the game files are, and whether the server takes sign-ups. The address can be:

- the IP the setup printed (`192.168.1.20`), or the Mac's `.local` name (Android resolves it);
- `host:3724`, the same as `host`;
- `host:<another port>`, for a data host on that port;
- `http://…` or `https://…`, for a data host behind a reverse proxy.

The app then saves the server with the account, keeps the password in the Android Keystore, and
logs in by itself after the download's restart. It enters the world at the address you typed,
whatever address realmd hands out, so a Mac whose address changed still works once you type the
new one. Against a server without `server.json` (another 1.12.1 core, an older data host), the
app falls back to the game files only, or to its manual server form.

## Accounts

1.12 has no sign-up of its own, so the data host adds one.

- **On by default, for phones on your network** (`INILLA_SIGNUP=lan` in `.env`). The app makes a
  name and a password and shows them; the phone works out the SRP6 salt and verifier itself, so
  the password never leaves it. The server writes the new account with a database user that can
  only add accounts.
- **Off**: `INILLA_SIGNUP=off` in `.env`, then run the setup again (or `docker compose up -d`).
  Accounts are then made on the console (below). `open` takes sign-ups from anywhere, for a
  server on the internet, and only makes sense with a token.
- **With a token** (`INILLA_DATA_TOKEN`), sign-ups need it too, as the game files do.
- **Limits**: 5 new accounts in 10 minutes and 20 a day per address, 30 an hour in all. Behind
  Docker Desktop and OrbStack every phone arrives from the same address, so there the per-address
  limits count all phones together.
- **Game masters**: no account made in the app is one. Make yours one on the console (it applies
  at the account's next login):

  ```sh
  cd ~/Vanillish-server
  docker compose exec mangosd vmangos-console account set gmlevel ALICE 6
  ```

- **On the console**, as before: `docker compose exec mangosd vmangos-console account create alice s3cretpass`.

## Everyday use

From `~/Vanillish-server` (or this folder):

```sh
docker compose ps                        # what runs (db-init and extract show "Exited (0)")
docker compose stop                      # stop the realm (characters are saved)
docker compose start                     # start it again
docker compose logs -f mangosd           # the world server's log
docker compose restart mangosd           # after editing config/mangosd.conf
docker compose exec mangosd vmangos-console server info
./setup-mac.sh                           # update (or ./setup-linux.sh); also a new address
./setup-mac.sh --uninstall               # remove it (asks before deleting anything)
```

- **Server settings** (rates, the message of the day, the content patch, …): edit
  [`config/mangosd.conf`](config/mangosd.conf) and [`config/realmd.conf`](config/realmd.conf):
  `Key = Value` overrides, applied over vmangos' defaults on every start.
- **The realm** (name, type) and the rest: `.env` ([`.env.example`](.env.example) explains every
  setting), then run the setup again or `docker compose up -d`.

Backups, the game files' volume, pathfinding, German, updating vmangos and troubleshooting are in
**[docs/OPERATIONS.md](docs/OPERATIONS.md)**. How the pieces fit together, and why, is in
**[docs/DESIGN.md](docs/DESIGN.md)**.

## Only the data host, for a server you already run

Any 1.12.1 core with Warden off works with the app (vmangos, cMaNGOS). To give its players the
game files and pfQuest without this realm, run the data host alone:

```sh
WOW_DATA_DIR="/path/to/WoW 1.12.1/Data" docker compose -f compose.datahost.yaml up -d
```

It takes no sign-ups (there is no database for it to write to); `WOW_DATA_DIR_DEDE`,
`INILLA_DATA_TOKEN` and the `addons/` folder work as in the full stack.

## Layout

| path | what |
|---|---|
| `setup-mac.sh`, `setup-linux.sh` | the one-command setup (the Linux one hands over to the first, which carries both) |
| `compose.yaml` | the stack |
| `.env.example` | every setting, documented |
| `config/` | overrides for `mangosd.conf` and `realmd.conf` |
| `docker/vmangos/` | the vmangos image: `Dockerfile` (compiled at a pinned commit, with the world database's dump) and `bin/` (the roles: `inilla-db-init`, `inilla-extract`, `inilla-realmd`, `inilla-mangosd`, and `vmangos-console`) |
| `datahost/` | the data host (Rust): `server.json`, the game files, the addons, the sign-up |
| `addons/` | your own addons for the app, per language (see [addons/README.md](addons/README.md)) |
| `compose.datahost.yaml` | only the data host, beside a server you already run |
| `docs/` | operations and design notes |

## A word on copyright

The archives in your game folder belong to Blizzard. Nothing of theirs is in this repository, in
any image or in any build context (`.dockerignore` keeps `*.MPQ` and `Data/` folders out): the
setup copies them from your folder into a Docker volume on your own machine, and the data host
serves them on your own network, for your phone. Serving them to anyone else means distributing
Blizzard's files, so share the server only with people who own the game, and set a token
(`INILLA_DATA_TOKEN`) before it is reachable from outside your home. Vanillish is an independent
fan project, not affiliated with or endorsed by Blizzard Entertainment.

## Credits and licences

- This server is [iNilla-server](https://github.com/Martze96/iNilla-server), the companion of the
  iNilla iOS app, taken over for Vanillish; the iNilla app works with it too.
- [vmangos](https://github.com/vmangos/core) is licensed under the GPLv2 and built unmodified from
  its upstream source at the commit `compose.yaml` pins; each image is labelled with that commit,
  and its source is at `https://github.com/vmangos/core/tree/<commit>`.
- [pfQuest](https://github.com/shagu/pfQuest) (MIT) comes with the data host.
- Our own code is MIT OR Apache-2.0 ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
