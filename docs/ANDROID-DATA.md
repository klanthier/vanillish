# The game data on Android

Vanillish reads the game — art, models, maps, sounds, the stock interface — from **your own**
World of Warcraft **1.12.1 (build 5875)** client, as iNilla and benilla do. The app contains no
Blizzard files. It needs the `.MPQ` archives at the root of the client's `Data` folder, about 5 GB
([DATA.md](DATA.md) lists them, and the languages).

On the phone they end up in Vanillish's folder in shared storage,
`Android/data/io.github.klanthier.vanillish/files/.vanillish/Data`, and survive restarts and
updates. Uninstalling deletes them. You do this once.

**Vanillish's folder** is the app's own corner of the phone's storage. A PC sees it over USB; the
phone's own file managers cannot open `Android/data` (Android blocks it), which is why the phone
has its own way in (Way 3). It exists once Vanillish has been opened, and the setup screen names
it:

| build | package | the folder, as a PC shows it |
|---|---|---|
| a release, or a CI build of the `release` variant | `io.github.klanthier.vanillish` | Internal storage → Android → data → `io.github.klanthier.vanillish` → files |
| a `debug` build | `io.github.klanthier.vanillish.debug` | Internal storage → Android → data → `io.github.klanthier.vanillish.debug` → files |

A debug build is a separate app beside the release one: it has its own folder and needs its own
copy of the data.

## Way 1 — download it from your server

The simplest: Vanillish's first screen, **Connect**, takes the game files from a server of your
own. No server yet? [`server/`](../server/README.md) sets one up in one command, on a Mac with
Apple silicon (or Linux, or Windows with WSL): it copies the archives from your WoW folder into
Docker once, and prints the address to type. Its realm, its game files and its sign-up then come
from that one address.

1. On the phone, on the same Wi-Fi, open Vanillish, type that address under **Server address**, and
   tap **Connect**.
2. The download starts by itself on Wi-Fi: about 5 GB, resumable, every archive checked against its
   SHA-256, the server's addons (pfQuest) installed after the archives. A language to pick comes
   first when the server has several, and an access code only if the server's owner set one. On
   mobile data, or a hotspot marked metered, the screen says so and waits for **Download the game
   (… GB)**.
3. Meanwhile, **Create account** on the right (or **I have an account**).
4. When it is done, **Restart Vanillish now**: Vanillish starts again and logs in by itself.

[ANDROID.md](ANDROID.md#quick-connect) has the rest: what the address may be, the sign-up, the
access code, a server that does not describe itself. On Android:

- the archives go straight into `.vanillish/Data`, with no copy afterwards;
- plain `http://` on your own network works: the download is the client's own HTTP, not Android's
  (whose rule against clear-text traffic applies to Java apps);
- the screen stays on while the download runs; keep the app open.

**By hand**, for any data host (an iNilla server, one behind a proxy, one Connect did not find):
on Connect tap **Other ways to get the game files…**, and on the setup screen, under **2 ·
Download it from your server**, enter `http://<that machine>:8099/` (filled in from the current
server; an access token only if the host set one), **Check server**, then **Download** — as on iOS
([DATA.md](DATA.md#way-2--download-it-from-your-server)), a language to pick when the server has
several. That download does not look at the network: over mobile data, 5 GB counts against your
plan, and nothing warns you.

## Way 2 — copy the folder in from a PC

1. Open Vanillish once (that makes its folder), and on Connect tap **Other ways to get the game
   files…**: the setup screen opens.
2. Connect the phone with a USB-C cable, unlock it, and choose **File transfer** in the USB
   notification.
3. On the computer, open **Internal storage → Android → data → io.github.klanthier.vanillish →
   files**, and drag in the client's `Data` folder, or the whole `World of Warcraft` folder.
   - **Windows**: File Explorer, under the phone.
   - **macOS** has no MTP built in: use an MTP app such as OpenMTP.
   - **Linux**: GNOME Files or Dolphin (MTP).
   - **Developers**: `adb push "World of Warcraft/Data" /sdcard/Android/data/io.github.klanthier.vanillish/files/`
4. On the phone, tap **Check for copied data** — or **Check again after restart**, or just restart
   Vanillish.

A complete folder found in Vanillish's folder, at most three levels deep (`Data`,
`World of Warcraft/Data`, `WoW 1.12.1/World of Warcraft/Data`), is then **moved** into place — a
rename on the same storage, instant, no second copy. A launch then starts with the game data in
place: on Connect when there is no server yet, else at the launcher or the automatic login. After
Check for copied data, tap **Restart Vanillish now**.

The move waits for the copy to be finished: every archive untouched for 15 seconds, and each one
whole (its MPQ header, and the tables it points to, inside the file). While a copy is still
running, Check for copied data says the folder "changed in the last few seconds — a copy may still
be running", and a restart leaves the folder alone; an archive cut short is named, to be copied in
again.

## Way 3 — Import from folder… on the phone

For a game already on the phone (a download, a Quick Share, a copy from a PC into Download), or on
a USB-C stick or drive:

1. Put it in a folder of its own, for example `Download/WoW`. Android's folder picker cannot pick
   the Download folder itself (nor the storage's top level, nor `Android/data`); a folder inside
   Download is fine.
2. On the setup screen (Connect → **Other ways to get the game files…**), tap **Import from
   folder…** — Connect has it too, for a server that shares no game files. The picker opens in
   Download: pick that folder, the client's `Data` folder, or the `World of Warcraft` folder above
   it.
3. Vanillish looks through it — three folders deep, never into `Interface`, `WTF`, `Cache`,
   `Screenshots`, `Logs` or `Errors`, at most 400 folders — takes the folder with the most
   complete set of archives, checks the free space, and copies them in with a progress bar.
   - **Pause** stops between two 4 MiB chunks. **Resume copying from \<folder\>** continues where
     it stopped — after a Pause, a crash, a power cut, or Android ending the app in the
     background: the picker's permission and the partly copied files (hidden, `.<archive>.copy`)
     survive. A part continues only from the very file it was copied from (the same folder, size
     and time, and the same first bytes), and only from what was surely on the phone's storage (it
     is synced every 64 MiB); anything else starts that archive over. A download's partly fetched
     files are never continued by a copy.
   - An archive already there at its size is kept, not copied again — unless its source changed
     after it was copied, which is copied again.
   - Each archive's size and MPQ header are checked before it takes its name, so an interrupted
     copy never leaves a half archive for the game to open.
   - A folder something is still writing into is refused, since a file being fetched can already
     have its full size and header (a torrent client sizes its files first): if any file changed
     in the last 15 seconds, the message is "\<folder\> changed in the last few seconds — a copy or
     download may still be running. Let it finish, then try again.", and Resume tries again.
4. It is a **copy**: the original folder stays where it was, and the message at the end says how
   much deleting it frees.

The copy needs room for the archives once more (about 5 GB). The free space shows beside the
button, in red when it is short, and a copy that would not fit does not start ("Not enough space:
… GB needed, … GB free."). Keep Vanillish open while it runs; the screen stays on by itself.

## Afterwards

The game opens its archives when it starts: when new data has arrived, **Restart Vanillish now**
starts it again by itself, in a fresh process. If it does not come back, open it from the apps
screen.

The setup screen is also there with the data complete (Settings → Debug → **Game data…**). Once an
import, a copy or a download from there has added, removed or replaced an archive, it offers
**Restart Vanillish now** instead of **Continue**: the running game still holds the old archives'
tables, and would read the new files with them.

## Things to know

| where | what to know |
|---|---|
| mobile data | Connect says so and waits for a tap before 5 GB; the setup screen's download does not look. |
| a Mac | has no MTP for a USB copy: OpenMTP (or another MTP app), or `adb push`; or serve the files from it ([Way 1](#way-1--download-it-from-your-server)). |
| the Download folder | cannot be picked itself: pick a folder inside it. |
| a debug build | its own package (`….vanillish.debug`), its own folder, its own copy of the data. |
| the phone's file managers | cannot open `Android/data`: from the phone, Import from folder…; from a PC, USB. |
| updating | keeps the data, as long as the new APK is signed with the same key; uninstalling deletes it ([ANDROID-BUILD.md](ANDROID-BUILD.md#signing)). |
| the PC's clock | a USB copy stamped with a time in the future (another time zone, a clock off) reads as "still changing" until that time passes. |
| shared storage at launch | right after a system update Android can take a moment to open `Android/data`. If it has not after about three seconds, the setup screen says "Shared storage is not available right now. Close Vanillish and open it again." and starts no download or import; nothing is lost. |

Copyright: the archives are Blizzard's; [DATA.md](DATA.md#a-word-on-copyright) says the rest.
