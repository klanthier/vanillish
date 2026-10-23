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

   Under it, the run that made the code for the app's [admin menu](#the-admin-menu) prints the
   code, and only that run does (the next ones say where it is, or why the menu cannot use the
   one in `.env`):

   ```
   Admin code (in Vanillish: the N button, then Settings → Server): ABCD-EFGH-JKLM-NPQR
   It sets the realm's rates and gives gold: keep it to yourself (ADMIN_CODE in ~/Vanillish-server/.env).
   ```

4. **On the phone**, on the same Wi-Fi: open Vanillish, type that address under **Server address**,
   and tap **Connect**. The app downloads the game files from your Mac (5.4 GB, once), you tap
   **Create account**, and you play. The game files can download while the Mac is still loading
   the world: the setup says when the phone can start.

Run the same command again to update the server, or when the Mac has a new address. It keeps
your passwords, accounts, characters, game files, admin code and the rates the admin menu set,
and skips every step that is already done.
The server lives in `~/Vanillish-server`, and `~/Vanillish-server/setup-mac.sh` is the same
script, for next time.

**What it asks, and what it checks:**

| step | what happens |
|---|---|
| the server's files | downloaded from this repository into `~/Vanillish-server`; your `.env`, `config/` and `addons/` survive an update |
| this Mac | Apple silicon, the macOS version |
| Docker | Docker Desktop, OrbStack or Colima, started for you if it is not running; a warning if it has less than 4 GB of memory |
| the address | the Mac's address on your network (the Wi-Fi or Ethernet one, never a VPN's), and its `.local` name |
| `.env` | new database passwords (with `openssl`) and the admin menu's code, made once; the address refreshed on every run |
| your game files | a Finder window to pick the WoW folder (or `--data "/path/to/World of Warcraft"`). It needs the 11 archives of a 1.12.1 client (`dbc`, `fonts`, `interface`, `misc`, `model`, `patch`, `sound`, `speech`, `terrain`, `texture`, `wmo`); an English (enUS) client is the tested one. Only the archives the game opens are copied, into the Docker volume `vanillish-wowdata`, and every one's size is checked against the original. |
| creature pathfinding | optional, asked once: monsters walk around obstacles instead of through them. Generated in the background after the first start, about 1.5 to 3 hours on an M4; you can play meanwhile. |
| start | the prebuilt images, then the first start, with its progress: the database (a minute), the server's maps from your game files (3 to 5 minutes), the world (1 to 2 minutes) |

Closing the Terminal window or pressing Ctrl-C during the first start does not stop it: the server
finishes by itself. `setup-mac.sh --help` lists the options (`--data`, `--ip`, `--reimport`, `--mmaps`,
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
                         # and ADMIN_CODE, for the admin menu (openssl rand -hex 10)
```

- **The game files**: either set `WOW_DATA_DIR` in `.env` to your client's `Data` folder (a
  bind mount: no copy, fine on Linux and Windows), or copy the archives into the volume once:

  ```sh
  docker volume create vanillish-wowdata
  COPYFILE_DISABLE=1 tar -cf - -C "/path/to/World of Warcraft/Data" dbc.MPQ fonts.MPQ \
      interface.MPQ misc.MPQ model.MPQ patch.MPQ sound.MPQ speech.MPQ terrain.MPQ texture.MPQ wmo.MPQ |
    docker run --rm -i -v vanillish-wowdata:/d alpine:3 tar -xf - -C /d
  ```

  Add your client's `patch-2.MPQ` (and any other `patch-N.MPQ`) and `speech2.MPQ` to the list when
  it has them. In the volume the archives must carry these stock names, in this case (the
  extractors open them by name, and Linux is case-sensitive). `COPYFILE_DISABLE=1` keeps a Mac's
  `._` metadata files out of the copy.
- **Start**: `docker compose up -d`. Compose downloads the prebuilt images; it builds one here only
  when the registry does not have it (your own `VMANGOS_COMMIT`, or an unpublished fork), which
  takes about 10 minutes more.

## What runs

| service | port | what the app uses it for |
|---|---|---|
| `realmd` | 3724 | the logon |
| `mangosd` | 8085 | the world. Beside it, in the same container, the admin menu's agent on 8091: on compose's own network, for the data host, never published |
| `datahost` | 8099 | `server.json` (what the app reads after you type the address), the one-time download of the game files (resumable, SHA-256 checked; pfQuest comes with them), the sign-up, and the [admin menu](#the-admin-menu)'s requests, which it checks and hands on to mangosd |
| `db` | — | MariaDB for vmangos. Never published. |
| `db-init`, `extract` | — | one-shot jobs on every start: the database (and its upgrades), and the server's maps, extracted once from your game files. They exit when done. |

The server is vmangos, built from source at a pinned commit, with one small patch of Vanillish's
(the admin menu's player speed) and Warden off (the app does not implement it). The images are
published as `ghcr.io/klanthier/vanillish-vmangos` and `ghcr.io/klanthier/vanillish-datahost`, for
linux/amd64 and linux/arm64, each built natively. **No image contains anything from Blizzard**:
your game files stay in a Docker volume on your own machine.

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
- **Limits**: 5 sign-up attempts in 10 minutes and 20 a day per address, 30 an hour in all. Behind
  Docker Desktop and OrbStack every phone arrives from the same address, so there the per-address
  limits count all phones together.
- **Game masters**: no account made in the app is one. Make yours one on the console (it applies
  at the account's next login):

  ```sh
  cd ~/Vanillish-server
  docker compose exec mangosd vmangos-console account set gmlevel ALICE 6
  ```

- **On the console**, as before: `docker compose exec mangosd vmangos-console account create alice s3cretpass`.

## The admin menu

The app has a menu for the server's owner. While everyone plays, it sets the realm's
**experience** (kills, quests, exploration), its **item drops** (by quality, grey to orange), the
**gold** from loot and quest rewards and every player's **speed**, and it **gives gold** to
players. A change is live at once, for everyone (the drops from the next kill), and kept when the
server restarts.

- **In Vanillish**: the **N** button at the top of the screen, then **Settings → Server**, for the
  server you connected to. It asks for the admin code once, and keeps it on the phone, sealed with
  an Android Keystore key ([docs/ANDROID.md](../docs/ANDROID.md#the-server-admin-menu)).
- **The code** is `ADMIN_CODE` in `.env`. The setup makes one (16 letters, `ABCD-EFGH-JKLM-NPQR`)
  and prints it at the end of the run that made it; to see it again,
  `grep ADMIN_CODE ~/Vanillish-server/.env`. Whoever has it sets the rates and hands out gold:
  keep it to yourself. It never travels: the app signs each request with it. Your own: a random
  one, `openssl rand -hex 10` (or the setup's), then `docker compose up -d`; a code picked by hand
  is refused, and the menu stays off. An empty `ADMIN_CODE=` means no admin menu, and the setup
  leaves it empty.
- **Where it answers**, `INILLA_ADMIN` in `.env`: `lan` (the default) phones on a private network,
  `open` anyone who reaches port 8099 (with the code), `off` nobody, whatever the code. As with
  sign-up, Docker Desktop and OrbStack (and a reverse proxy) show the data host every phone at a
  private address, so there `lan` lets anyone who reaches 8099 try: **the code is the only real
  guard.** After ten wrong codes in ten minutes, an address's wrong codes get a "too many" answer
  (check the code), but the right code always gets in, so that limit stops no one who keeps
  guessing: the code's randomness does.
- **Its rates win over [`config/mangosd.conf`](config/mangosd.conf)**, across restarts, until
  **Server defaults** in the app puts every rate back to that file's. They are kept in the Docker
  volume `admin`, which `docker compose down -v` deletes. Applying reloads the world server's
  settings, which also puts the message of the day and the player limit back to the file's.
- **Loot, honestly**: vmangos multiplies the chance of an item that drops on its own. Bosses and
  dungeons often drop one item from a list (a loot group), and that roll takes no rate, so their
  loot does not change. **Shared loot tables** is how often a table several creatures share is
  rolled at all. White items include quest items.
- **Gold arrives by mail**, "A gift from the realm", to characters online or not, by name (up to
  50 at a time) or everyone online, up to 1000 gold each: they collect it at any mailbox. A name
  counts as sent when the world server answers with its own "mail sent" sentence, in the realm's
  language; otherwise the app says "maybe sent": check a mailbox before sending to it again.
- **Player speed needs this stack's vmangos image**, which carries Vanillish's patch for it
  ([below](#credits-and-licences)): walking, running, swimming and mounted, ×0.75 to ×2 in the app
  (taxi flights keep their own speed). On an older image, or one built from a `VMANGOS_COMMIT` the
  patch does not apply to, the app has no speed setting, and a speed it set before stays saved,
  unapplied, until **Server defaults** drops it.
- **What runs it**: the data host checks each request's signature and hands it on to the admin
  agent, `vanillish-admin`, inside the mangosd container (port 8091 on compose's own network, never
  published). The agent writes the rates and drives mangosd's console (`reload config`, `announce`,
  `send money`); its lines are in `docker compose logs mangosd` (`[admin] …`).

Rotating the code, turning the menu off and where the rates are:
[docs/OPERATIONS.md](docs/OPERATIONS.md#the-admin-menu).

## Everyday use

From `~/Vanillish-server` (or this folder):

```sh
docker compose ps                        # what runs (db-init and extract show "Exited (0)")
docker compose stop                      # stop the realm (characters are saved)
docker compose start                     # start it again
docker compose logs -f mangosd           # the world server's log
docker compose restart mangosd           # after editing config/mangosd.conf
docker compose exec mangosd vmangos-console server info
grep ADMIN_CODE .env                     # the admin menu's code
./setup-mac.sh                           # update (or ./setup-linux.sh); also a new address
./setup-mac.sh --uninstall               # remove it (asks before deleting anything)
```

- **Server settings** (rates, the message of the day, the content patch, …): edit
  [`config/mangosd.conf`](config/mangosd.conf) and [`config/realmd.conf`](config/realmd.conf):
  `Key = Value` overrides, applied over vmangos' defaults on every start. The XP, loot, gold and
  speed rates are also the [admin menu](#the-admin-menu)'s, live: a rate it set wins over this file
  until its **Server defaults**.
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

It takes no sign-ups (there is no database for it to write to) and has no admin menu (that needs
this stack's mangosd); `WOW_DATA_DIR_DEDE`, `INILLA_DATA_TOKEN` and the `addons/` folder work as
in the full stack.

## Layout

| path | what |
|---|---|
| `setup-mac.sh`, `setup-linux.sh` | the one-command setup (the Linux one hands over to the first, which carries both) |
| `ci/setup-test.sh` | the setup's own tests (`bash ci/setup-test.sh`, under any bash from macOS' 3.2 on) |
| `ci/admin_test.py` | the admin agent's own tests (`python3 -I ci/admin_test.py`, standard library only) |
| `compose.yaml` | the stack |
| `.env.example` | every setting, documented |
| `config/` | overrides for `mangosd.conf` and `realmd.conf` |
| `docker/vmangos/` | the vmangos image: `Dockerfile` (compiled at a pinned commit, with the world database's dump), `patches/` (Vanillish's patch to vmangos: the player speed rate) and `bin/` (the roles: `inilla-db-init`, `inilla-extract`, `inilla-realmd`, `inilla-mangosd`; `vmangos-console`; and `vanillish-admin`, the admin menu's agent beside mangosd) |
| `datahost/` | the data host (Rust): `server.json`, the game files, the addons, the sign-up, the admin menu's door |
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
- [vmangos](https://github.com/vmangos/core) is licensed under the GPLv2. It is built from its
  upstream source at the commit `compose.yaml` pins, with one small patch of Vanillish's, which adds
  the player speed rate (`Rate.MoveSpeed.Player`): `docker/vmangos/patches/`, GPL-2.0-or-later like
  the code it changes. Each image is labelled with that commit and carries the patches it applied in
  `/opt/vmangos/patches`; the source it runs is `https://github.com/vmangos/core/tree/<commit>`
  with those patches.
- [pfQuest](https://github.com/shagu/pfQuest) (MIT) comes with the data host.
- Our own code is MIT OR Apache-2.0 ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
