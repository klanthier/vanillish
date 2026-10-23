# Operations

Everything here runs in the server's folder, where `compose.yaml` and `.env` are:
`~/Vanillish-server` after the one-command setup, or `server/` in a clone of this repository.
macOS Terminal, Linux and WSL shells, PowerShell and cmd take the commands as written. Git Bash
on Windows rewrites arguments that look like paths (`/opt/…` becomes `C:/Program Files/Git/opt/…`),
so there, prefix those commands with `MSYS_NO_PATHCONV=1`.

## The setup, again

`setup-mac.sh` (`setup-linux.sh` on Linux and WSL) is safe to run at any time, and it is the
answer to most changes:

| after | what the setup does |
|---|---|
| nothing (an update) | downloads the server's files again, keeping `.env`, `config/*.conf` and `addons/`; downloads newer images; starts what changed |
| the computer's address changed | writes the new address into `.env` (`REALM_ADDRESS`), and `db-init` into the realm list |
| an edit to `.env` | applies it (the same as `docker compose up -d`) |
| pathfinding finished in the background | restarts the world server so it uses it |
| a stop, a crash, a reboot without Docker at login | starts the server and follows it until it is up |

Its options: `--data DIR` (the WoW folder, without the dialog), `--reimport` (copy the game files
again), `--ip ADDRESS` (when the address it finds is not the one the phone reaches), `--mmaps` /
`--no-mmaps`, `--yes` (the default answer to every question), `--no-update` (keep the files that
are here), `--dry-run` (show, change nothing) and `--uninstall`.

## Accounts and GM levels

The app makes accounts itself (**Create account**) while `INILLA_SIGNUP` allows it:

| `INILLA_SIGNUP` | who may sign up |
|---|---|
| `lan` (the default in `compose.yaml`) | phones on a private network: 10/8, 172.16/12, 192.168/16, 100.64/10 (Tailscale), loopback, link-local, IPv6 ULA |
| `off` (the default in `compose.datahost.yaml`) | nobody: accounts are made on the console |
| `open` | anyone who reaches port 8099; set `INILLA_DATA_TOKEN` too |

After a change in `.env`, run the setup again or `docker compose up -d`. With `INILLA_DATA_TOKEN`
set, a sign-up needs the token as well. The limits are 5 sign-up attempts in 10 minutes and 20 a
day per address, and 30 an hour in all; past them the app says when to try again. On Docker Desktop
and OrbStack every phone reaches the server from the same gateway address, so `lan` cannot tell
your Wi-Fi from the internet there: never forward port 8099 to the internet without a token.

The sign-up never makes a game master, and it never changes an account that exists: a taken name
is refused. Everything else goes through mangosd's console, which `vmangos-console` drives one
command at a time:

```sh
docker compose exec mangosd vmangos-console account create alice s3cretpass
docker compose exec mangosd vmangos-console account set gmlevel alice 6
docker compose exec mangosd vmangos-console account set password alice newpass newpass
docker compose exec mangosd vmangos-console account onlinelist
docker compose exec mangosd vmangos-console account delete alice
```

vmangos stores names and passwords upper-cased, and the app sends them that way too, so
capitalisation does not matter when logging in.

GM levels: `0` player (the default), `1` moderator, `2` ticket master, `3` game master,
`4` basic admin, `5` developer, `6` administrator. A new level applies at the account's next
world login. GM commands are typed in chat with a leading dot (`.gm on`, `.tele Stormwind`,
`.help`). On the console, leave the dot off:

```sh
docker compose exec mangosd vmangos-console server info
docker compose exec mangosd vmangos-console announce Server restarts in 10 minutes
docker compose exec mangosd vmangos-console help
```

Given no command, it reads them from stdin, one per line (a blank line is skipped), and exits
non-zero only when one of them failed:
`docker compose exec -T mangosd vmangos-console < commands.txt`.

The world has to be loaded first. When `docker compose ps` shows mangosd as `healthy`, it is. A
command waits up to 20 s for it, then gives up with nothing sent ("the world is still loading").
Each run of `vmangos-console` takes 2 s before its first command even then: the world port has to
have listened that long.

## Settings

| what | where | apply with |
|---|---|---|
| realm name and type (pve/pvp/rp/rppvp), ports, sign-up, the data token, the admin code (`ADMIN_CODE`) and where the admin menu answers (`INILLA_ADMIN`) | `.env` | the setup, or `docker compose up -d` |
| the address the phone reaches | `.env` (`REALM_ADDRESS`), written by the setup | the setup |
| the XP, item drop, gold and player speed rates | the app's admin menu (Settings → Server), or `config/mangosd.conf` | **Apply** in the app, live; the file with `docker compose restart mangosd` |
| MOTD, content patch, player limits, the other rates, … | `config/mangosd.conf` | `docker compose restart mangosd` |
| logon throttling, … | `config/realmd.conf` | `docker compose restart realmd` |

The override files hold `Key = Value` lines, applied over vmangos' defaults on every start. The
stack manages the database strings, paths, ports, the console, Warden and the vmap/mmap switches
itself. An override file wins even over those, so leave them out unless you mean it. To see every
key with its documentation:

```sh
docker compose exec mangosd cat /opt/vmangos/etc/mangosd.conf.dist
```

To see what mangosd actually runs with: `docker compose exec mangosd cat /run/vmangos/mangosd.conf`
(the admin menu's rates included; `/run/vmangos/mangosd.base.conf` is the same without them).

A rate the admin menu set wins over `config/mangosd.conf`, across restarts, until the menu's
**Server defaults**. Its **Apply** reloads mangosd's settings (`reload config`), which puts the
message of the day and the player limit back to the file's as well: a MOTD set in the game with
`.server set motd` is lost then.

The database passwords in `.env` were made for this machine. Keep `DB_ROOT_PASSWORD` as it is:
MariaDB keeps the one it was created with. `db-init` gives the `mangos` user the password in
`.env` on every start, and the sign-up's user (`vanillish_signup`) `SIGNUP_DB_PASSWORD`.

## The admin menu

The app's admin menu ([README](../README.md#the-admin-menu)) answers while `ADMIN_CODE` in `.env`
holds a random code: the setup's (16 letters, `ABCD-EFGH-JKLM-NPQR`), or 20 or more hex digits
(`openssl rand -hex 10`); dashes, spaces and case do not count. A code picked by hand, or the
example's `change-me`, leaves the menu off: the setup warns, and the data host's log says why.

```sh
grep ADMIN_CODE .env                                       # the code
docker compose exec mangosd cat /vmangos/admin/rates.conf  # the rates the menu set (none: no file)
docker compose logs mangosd | grep '\[admin\]'             # the agent: each request, each change
docker compose logs datahost | grep ' admin '              # the door: each request and its status
```

**Where the rates are.** `rates.conf`, in the Docker volume `vanillish_admin` (mounted at
`/vmangos/admin` in mangosd), `Key = value` lines. They outlive restarts, `docker compose down`
and updates; `docker compose down -v` deletes them with the database. The database backup
([below](#backups)) does not hold them. They are a few lines, so to keep them:

```sh
docker compose exec mangosd cat /vmangos/admin/rates.conf > admin-rates.conf
# and back, later:
docker compose exec -T mangosd sh -c 'cat > /vmangos/admin/rates.conf' < admin-rates.conf
docker compose restart mangosd
```

A line the menu does not set, or whose value is not a plain number in its range, is dropped when
mangosd starts, with a line in its log.

**A new code** (one somebody else has seen): set `ADMIN_CODE` in `.env` to a random one
(`openssl rand -hex 10`) and run `docker compose up -d`; or set it to `change-me` (any case,
dashes or not) and run the setup again, which makes a new one, prints it and applies it (during
the creature pathfinding it applies it on the run after). Either way the data host and mangosd
restart with it, so the world restarts too: the players are logged out for a minute or two. In
the app, the old code is refused ("That is not this server's admin code.", or "Too many wrong codes
from here lately: …" once ten wrong ones came from that address), forgotten, and the tab asks for
the new one.

**Turning the menu off**: `INILLA_ADMIN=off` in `.env`, then `docker compose up -d`; only the data
host restarts. The rates the menu set stay applied: tap **Server defaults** first to drop them. An
empty `ADMIN_CODE=` turns it off everywhere instead (mangosd restarts): the rates stay in the volume,
unapplied, mangosd's log counts them, and a code brings them back. The setup leaves an empty code
empty.

**Where it answers**: `INILLA_ADMIN=lan` (the default) from the private addresses the sign-up's
`lan` takes ([above](#accounts-and-gm-levels)), `open` from anywhere 8099 reaches. On Docker Desktop
and OrbStack every phone arrives from the same gateway address, so `lan` cannot tell your Wi-Fi from
the internet there, and the code is the only real guard. After ten wrong codes in ten minutes,
the wrong codes from that address get a "too many" answer instead, which promises no wait: the app
says to check the code. The right code always gets in, so the limit stops no one who keeps
guessing: the code's randomness does.

## The game files

The setup copies your client's archives into the Docker volume `vanillish-wowdata`, once. The
server's map extraction and the data host read them from there, read-only. The volume outlives
everything else: `docker compose down -v` (a fresh realm) leaves it alone.

```sh
docker run --rm -v vanillish-wowdata:/d:ro alpine:3 ls -l /d     # what is in it
./setup-mac.sh --reimport                                         # copy them again (another client)
docker volume rm vanillish-wowdata                                # delete them (the server stopped)
```

Only the archives the client opens travel: the 11 required ones, `patch-?.MPQ` and `speech2.MPQ`,
renamed to their stock spelling (`DBC.MPQ` becomes `dbc.MPQ`). Repack extras (`base.MPQ`,
`backup.MPQ`) and the `Interface`, `WTF` and `Cache` folders stay behind.

On a Mac, a WoW folder in Desktop, Documents, Downloads or on an external drive makes macOS ask
whether Terminal may open it: allow it. If it was refused, allow it in System Settings → Privacy
& Security → Files and Folders, then run the setup again. A folder in iCloud Drive with *Optimize
Mac Storage* is downloaded as it is read, slowly.

**A folder instead of the volume** (Linux, Windows): set `WOW_DATA_DIR` in `.env` to the client's
`Data` folder, and `docker compose up -d`. Compose then bind-mounts it read-only, with no copy;
the archives must then be spelled as above. On a Mac, keep the volume: Docker Desktop's file
sharing insists on a file's original case, and a bind-mounted folder that moves or whose drive is
unplugged stops the server.

## Movement maps (mmaps)

Without mmaps, creatures chase you in straight lines, through trees and up cliffs. The setup asks
once (`EXTRACT_MMAPS` in `.env`); with `1`, the first start brings the realm up without them and
then generates them in the background: about 1.5 to 3 hours on an Apple M4, longer on an older
or smaller machine, with every core busy. You can play meanwhile. mangosd uses them from its next
start: run the setup again once they are done (it restarts mangosd for you), or
`docker compose restart mangosd`.

```sh
docker compose logs -f extract       # how far it is
```

While they are being generated, `docker compose up -d` waits for them to finish before it
returns; Ctrl-C there is harmless. A reboot in the middle stops the generation: the next run of
the setup starts it again.

By hand, on any machine: set `EXTRACT_MMAPS=1` in `.env`, then

```sh
docker compose up -d --no-deps extract    # hours; the realm keeps running
docker compose restart mangosd            # once `docker compose ps -a extract` shows "Exited (0)"
```

## Addons for the app

The data host hands the app **pfQuest** (English and German builds, from the image) and whatever
you put in `addons/` (see [addons/README.md](../addons/README.md)). The app installs them with the
game files, and **Install / update addons** on its Game data screen fetches changes later. After
changing `addons/`, run `docker compose restart datahost`. The setup never touches what you add
there.

## German

The server side is German already: vmangos' world database carries German quest texts, items,
creatures and spells, and it sends them to every client that logs in as German. The app logs in
in the language of its game files, so German needs a German 1.12.1 client's `Data` folder on the
data host:

1. Build a German 1.12.1 client on a Windows PC: install the German 1.1.2 client
   (`WoW-1.1.2.4125-deDE.iso`), then run the German `1.12.0` patch and the `1.12.0 → 1.12.1` patch
   on it. Blizzard's installers and patchers run on Windows 10 and 11.
2. In `.env`, set `WOW_DATA_DIR_DEDE` to that client's `Data` folder, then run
   `docker compose up -d`. The data host hashes it once (a minute or two) and then offers
   **Deutsch** next to English, with pfQuest's German build. (On a Mac, keep that folder under
   your home folder, so Docker can share it.)
3. On the phone, **Game data** → **Check server** → **Deutsch** → **Download**. The phone keeps one
   language at a time, so switching downloads that language's archives in place of the others.

The realm keeps its map data from the English client; it has no language.

The app's German is new and untested: it is built to read a German client's text, but nobody has
played it yet. Report what shows up in English, or empty.

## Backups

The world database can be rebuilt at any time. Accounts (`realmd`) and characters (`characters`)
cannot, so back those up:

```sh
docker compose exec db sh -c 'mariadb-dump -uroot -p"$MARIADB_ROOT_PASSWORD" --single-transaction --databases realmd characters > /tmp/vanillish-backup.sql'
docker compose cp db:/tmp/vanillish-backup.sql ./vanillish-backup.sql
```

To restore:

```sh
docker compose stop mangosd realmd
docker compose cp ./vanillish-backup.sql db:/tmp/vanillish-backup.sql
docker compose exec db sh -c 'mariadb -uroot -p"$MARIADB_ROOT_PASSWORD" < /tmp/vanillish-backup.sql'
docker compose up -d
```

`docker compose up -d` at the end also re-applies the realm row from `.env`, which the backup
overwrote. The admin menu's rates are not in the database: [The admin menu](#the-admin-menu) keeps
them.

## Updating

**The server**: run the setup again. It takes the latest files of this repository and the images
they name. An image's tag names the vmangos commit and a revision of the stack's scripts, so a new
one is downloaded only when something changed. `db-init` then applies the new migrations and
mangosd restarts on the new core.

**Another vmangos commit** (a newer core than the published images):

1. Back up (above).
2. Set `VMANGOS_COMMIT` in `.env` to a commit of
   [vmangos/core `development`](https://github.com/vmangos/core/commits/development).
3. Run `docker compose up -d`. No published image has that commit, so compose builds one here
   (about 10 minutes on an M4). Then `db-init` applies the new migrations and mangosd restarts on
   the new core.

The admin menu's player speed is Vanillish's patch to vmangos (`docker/vmangos/patches/`). The
build applies it when it still fits the commit, and otherwise builds without it, with a `WARNING:
0001-move-speed.patch does not apply` in its output: the app then has no speed setting, and a speed
the menu had set is left out of mangosd's config (its log says so) but kept in `rates.conf`, for an
image that has the patch again. The menu counts such a line among the rates it set, so **Server
defaults** shows, and drops it with the rest. The patches that went in:
`docker compose exec mangosd cat /opt/vmangos/vanillish-patches`.

A new image also means a new world-database dump, but only a *fresh* database imports it. An
existing one is brought forward by the core's migrations, which is how vmangos is meant to be
upgraded. If `db-init` warns that the database is newer than the core, the dump outran the core:
move `VMANGOS_COMMIT` forward.

On rare occasions vmangos changes its map format, and mangosd then refuses the old map files in its
log. To re-extract:

```sh
docker compose down
docker volume rm vanillish_serverdata
docker compose up -d
```

## Logs

```sh
docker compose logs -f mangosd          # console output
docker compose logs --since 1h realmd
docker compose logs datahost            # the data host, sign-ups and admin requests included
docker compose exec mangosd ls /vmangos/logs   # Server.log, DBErrors.log, Chat.log, weekly honor, …
```

## Moving to another machine, or a new address

- **A new LAN address:** run the setup again; on the phone, connect to the new address. A DHCP
  reservation in your router keeps the address from changing. On a Mac, set *Private Wi-Fi
  address* to Fixed or Off for your home network (System Settings → Wi-Fi → Details), or the
  reservation does not hold.
- **Another machine:** back up, run the setup there (with the same WoW folder), and restore the
  backup.

## Uninstall

```sh
./setup-mac.sh --uninstall      # or ./setup-linux.sh --uninstall
```

It asks before each step: the containers, the database, the map data and the admin menu's rates;
the images; the game files' volume; the folder with its `.env`.

## Troubleshooting

**The app cannot reach the server.** Open `http://<address>:8099/` in the phone's browser; it
should answer with one line of text.

- No answer: the phone and the computer are not on the same network (a guest Wi-Fi isolates its
  devices; mobile data is not your network), a VPN on the computer or the phone routes around the
  LAN, or a firewall stands between them:
  - **macOS**: *Block all incoming connections* must be off (System Settings → Network →
    Firewall → Options). If macOS asked whether Docker may accept incoming connections and the
    answer was Deny, remove that entry in the same Options list.
  - **Windows** (Docker Desktop): the firewall rule is for the program, not the port. The
    *Docker Desktop Backend* rule must allow inbound traffic on the profile your network uses.
    Check with `Get-NetFirewallRule -DisplayName "Docker Desktop Backend"`.
  - **Linux** (ufw): `sudo ufw allow 3724,8085,8099/tcp`.
  - **Docker Engine inside WSL**, rather than Docker Desktop, publishes its ports to Windows'
    localhost only, unless WSL's mirrored networking is on.
- An answer, but the app says the server refused the token: the data host has `INILLA_DATA_TOKEN`
  set. Enter it in the app, or empty it in `.env` and run `docker compose up -d`.
- The computer went to sleep: see "Keep it reachable" in the [README](../README.md).

**The app connects, but "Create account" is refused.**

- "This server does not take sign-ups from here": `INILLA_SIGNUP` is `off`, or `lan` and the phone
  is not on a private network.
- "That account name is taken": the name exists. If it is yours, choose **I have an account**.
- "Too many new accounts from here": the limits above; the app says when to try again.
- "The server is still starting": the database is not up yet; try again in a minute.

**Logon works, but entering the world fails or hangs.** The app enters the world at the address
you typed. An app that connected before your address changed still has the old one: connect again
with the new address. To see what realmd hands out (what older apps and desktop clients dial):

```sh
docker compose exec db sh -c 'mariadb -uroot -p"$MARIADB_ROOT_PASSWORD" -e "SELECT name, address, port, realmflags FROM realmd.realmlist"'
```

**The realm shows as offline.** mangosd is down or still loading: `docker compose ps` and
`docker compose logs mangosd`.

**The setup says an image could not be downloaded, and builds it.** The registry does not have
that tag: your own `VMANGOS_COMMIT`, a fork that has not published its images, or images still
private. The build takes about 10 minutes and happens once.

**"port is already allocated".** Another server holds 3724, 8085 or 8099, an iNilla server for
instance: stop it in its folder (`docker compose stop`) and run the setup again.

**`docker compose up` fails on `extract` or `db-init`.** Their logs say why:
`docker compose logs extract db-init`. Both are safe to re-run: `docker compose up -d`.

**Docker runs out of memory** (mangosd or the extractors stop with exit code 137): give Docker 4 GB
or more. Docker Desktop → Settings → Resources; OrbStack → Settings → System; Colima:
`colima stop && colima start --cpu 4 --memory 6`.

**`vmangos-console` says "the world is still loading".** mangosd has not opened its world port (for
2 s) within the command's 20 s: the world takes a minute or two to load after a start, and
`docker compose ps` shows mangosd `healthy` once it has. Nothing was sent, nor the commands after
it; try again then. "No prompt after 15 s": the world is up but has not got to the command yet (a
long command, or a world that hangs); it still runs, and its answer shows in
`docker compose logs mangosd`. "The console stayed busy for 20 s": the admin menu's agent held it (a
gold send to many players); try again. "An earlier command is still running": a command before it
(its own, or the admin menu's) got no prompt in time, or its `vmangos-console` was stopped first,
and the world has not finished it yet; try again later. "A command is one line of at most
254 bytes": a longer one, or one with a line break, is refused unsent (mangosd would run its rest as
a second command).

**The app's admin menu (Settings → Server) says:**

- "This server has no admin menu": `ADMIN_CODE` is empty, picked by hand (not the setup's 16
  letters or 20 or more hex digits) or still `change-me`, or the data host is older than the
  menu. Set it to `change-me` and run the setup again (or a random code and
  `docker compose up -d`). `docker compose logs datahost | grep 'admin menu'` says why it is off.
- "The admin menu answers only on the server's own network": `INILLA_ADMIN=lan`, and the phone is
  not on a private network (mobile data).
- "The admin menu is off on this server": `INILLA_ADMIN=off`.
- "Too many wrong codes from here lately": the code the app held is wrong, and ten wrong ones came
  from that address in ten minutes (on Docker Desktop and OrbStack, from any phone). The app has
  forgotten it: type the one in `.env`, which gets in whatever the count.
- "Maybe sent to …: the server did not say": the console's answer was not the realm's own "mail
  sent" sentence, or came too late. Check a mailbox before sending to them again.
- "No answer in time": the request reached the server and no answer came back (for a change or a
  gold send, a reverse proxy's error page in its place counts as none), so it may still have been
  done. Refresh (after a gold send, check a mailbox) before trying again.
- "The world server is starting…": mangosd, or the agent beside it, does not answer yet:
  `docker compose ps`, `docker compose logs mangosd`.
- "The data host and the world server disagree on ADMIN_CODE": `.env` changed, and only one of
  them has restarted since: `docker compose up -d`.

**Start over from nothing.** `docker compose down -v` deletes the database (accounts and
characters), the extracted server data and the admin menu's rates. The next `up` rebuilds the
first two. The game files' volume stays.
