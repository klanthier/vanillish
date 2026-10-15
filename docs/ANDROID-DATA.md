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
has its own way in (Way 2). It exists once Vanillish has been opened, and the setup screen names
it:

| build | package | the folder, as a PC shows it |
|---|---|---|
| a release, or a CI build of the `release` variant | `io.github.klanthier.vanillish` | Internal storage → Android → data → `io.github.klanthier.vanillish` → files |
| a `debug` build | `io.github.klanthier.vanillish.debug` | Internal storage → Android → data → `io.github.klanthier.vanillish.debug` → files |

A debug build is a separate app beside the release one: it has its own folder and needs its own
copy of the data.

## Way 1 — copy the folder in from a PC

1. Open Vanillish once; its setup screen opens.
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
rename on the same storage, instant, no second copy. At launch the app then goes straight to the
login screen; after Check for copied data, tap **Restart Vanillish now**.

The move waits for the copy to be finished: every archive untouched for 15 seconds, and each one
whole (its MPQ header, and the tables it points to, inside the file). While a copy is still
running, Check for copied data says the folder "changed in the last few seconds — a copy may still
be running", and a restart leaves the folder alone; an archive cut short is named, to be copied in
again.

## Way 2 — Import from folder… on the phone

For a game already on the phone (a download, a Quick Share, a copy from a PC into Download), or on
a USB-C stick or drive:

1. Put it in a folder of its own, for example `Download/WoW`. Android's folder picker cannot pick
   the Download folder itself (nor the storage's top level, nor `Android/data`); a folder inside
   Download is fine.
2. On the setup screen, tap **Import from folder…**. The picker opens in Download: pick that
   folder, the client's `Data` folder, or the `World of Warcraft` folder above it.
3. Vanillish looks through it — three folders deep, never into `Interface`, `WTF`, `Cache`,
   `Screenshots`, `Logs` or `Errors`, at most 400 folders — takes the folder with the most
   complete set of archives, checks the free space, and copies them in with a progress bar.
   - **Pause** stops between two 4 MiB chunks. **Resume copying from \<folder\>** continues where
     it stopped — after a Pause, a crash, or Android ending the app in the background: the picker's
     permission and the partly copied files survive.
   - An archive already there at its size is kept, not copied again.
   - Each archive's size and MPQ header are checked before it takes its name, so an interrupted
     copy never leaves a half archive for the game to open.
4. It is a **copy**: the original folder stays where it was, and the message at the end says how
   much deleting it frees.

The copy needs room for the archives once more (about 5 GB). The free space shows beside the
button, in red when it is short, and a copy that would not fit does not start ("Not enough space:
… GB needed, … GB free."). Keep Vanillish open while it runs; the screen stays on by itself.

## Way 3 — download it from your server

As on iOS ([DATA.md](DATA.md#way-2--download-it-from-your-server)): on the setup screen enter
`http://<that machine>:8099/` (filled in from the server you added; a token only if the host set
one), **Check server**, then **Download** — resumable, every archive checked against its SHA-256,
a language to pick when the server has several, its addons installed after the archives. On
Android:

- the archives go straight into `.vanillish/Data`, with no copy afterwards;
- plain `http://` on your own network works: the download is the client's own HTTP, not Android's
  (whose rule against clear-text traffic applies to Java apps);
- the screen stays on while the download runs; keep the app open;
- over mobile data, 5 GB counts against your plan, and nothing warns you.

## Afterwards

The game opens its archives when it starts: when new data has arrived, **Restart Vanillish now**
starts it again by itself, in a fresh process. If it does not come back, open it from the apps
screen.

## Things to know

| where | what to know |
|---|---|
| a Mac | has no MTP: OpenMTP (or another MTP app), or `adb push`. |
| the Download folder | cannot be picked itself: pick a folder inside it. |
| a debug build | its own package (`….vanillish.debug`), its own folder, its own copy of the data. |
| the phone's file managers | cannot open `Android/data`: from the phone, Import from folder…; from a PC, USB. |
| updating | keeps the data, as long as the new APK is signed with the same key; uninstalling deletes it ([ANDROID-BUILD.md](ANDROID-BUILD.md#signing)). |
| the PC's clock | a USB copy stamped with a time in the future (another time zone, a clock off) reads as "still changing" until that time passes. |

Copyright: the archives are Blizzard's; [DATA.md](DATA.md#a-word-on-copyright) says the rest.
