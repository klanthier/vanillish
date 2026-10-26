# Vanillish for Windows

Vanillish for Windows is the Vanillish client as a Windows PC game, with nothing inside the game
rewritten:

- Connect first;
- the launcher, with saved accounts and the automatic login;
- the server admin menu;
- iNilla's WoW Forever controller layout, with an Xbox controller first among the pads.

It is one `Vanillish.exe` in a zip ([WINDOWS-BUILD.md](WINDOWS-BUILD.md)). It needs:

- Windows 10 or 11 on x86-64;
- a GPU with Vulkan 1.1 or Direct3D 12;
- the game data of your own 1.12.1 client, which can be a client already on the PC, read where it
  is.

The keyboard and the mouse are benilla's, as on any desktop. The rest of this page is what
Vanillish adds on a PC.

## Install

1. Extract the zip `Vanillish-<version>-windows-x64.zip` to a folder of your own, for example
   `C:\Games\Vanillish`.
   - **Not** Program Files: Windows does not let a game write there.
   - **Not** inside a World of Warcraft folder: Vanillish never writes into an install.
2. Open `Vanillish.exe`. It is not code-signed, so SmartScreen may say "Windows protected your PC":
   choose **More info**, then **Run anyway**. Windows 11's Smart App Control, when on, blocks an
   unsigned exe outright.
3. To update, extract the new zip over the same folder. Your settings and data are elsewhere (below),
   so nothing is lost.

It opens full screen, borderless on the main monitor, as benilla does. To change that: the stock
Video options (*Esc → Video Options*: windowed, resolution), and they apply at the next start.

## The game data

The first screen is **Connect** ([ANDROID.md](ANDROID.md#quick-connect)). It shows the same flow
as on the phone, in the PC's words. There are three ways to bring the 1.12.1 game files:

- **Your own client, in place.** *Use my WoW folder…* opens the Windows folder dialog. Pick the
  World of Warcraft 1.12.1 folder (the one with `WoW.exe`) or its `Data` folder.
  - The folder must hold a whole 1.12.1 client, or the screen names what is missing.
  - From its next start Vanillish reads that folder where it is: no copy, and nothing is ever
    written into it. Downloads and imports refuse while it is in use.
  - *Stop using my WoW folder* goes back to Vanillish's own copy.
  - The choice is remembered in `benilla-config\inilla\install.toml`.
- **From your server.** Connect downloads the files into Vanillish's own `Data` folder (about
  5 GB, resumable, SHA-256 checked).
  - It starts by itself on an unmetered network.
  - A connection marked metered in *Windows Settings → Network & internet* waits for the button.
  - When the server runs on this same PC, type `localhost`.
- **A copy.** Either:
  - *Import from folder…* copies a client's archives in;
  - or copy (not move) its `Data` folder into the Vanillish folder yourself, so that it becomes
    `…\Vanillish\Data`, and restart Vanillish.

  A client's own folder (with `WoW.exe`) is never moved out of, not even from inside the
  Vanillish folder.

The game opens its archives when it starts. So after any of these, **Restart Vanillish now**
starts a new Vanillish and closes this one, and the saved account logs in by itself.

## Controllers

An Xbox controller is the reference pad: Xbox One, Series X|S, Elite, and the Xbox Wireless
Adapter. Connect it over USB or Bluetooth, or with the adapter. Windows reads it through
Windows.Gaming.Input, which benilla already compiles in through Bevy's gilrs input. The layout,
the trigger layers, cursor mode, the radial menu, the hotkey hints and *Settings → Controller*
are iNilla's ([INILLA.md](INILLA.md#controllers)).

- **No setting needed.** A controller works as soon as it is connected, mid-session too.
- **Button glyphs.** The bars show Xbox letters for an Xbox pad. A DualSense or a DualShock 4 gets
  PlayStation glyphs and a Switch Pro gets Nintendo ones: the glyphs follow the pad's USB vendor
  and product ids (`inilla_core::pads`), since the name Windows reports does not say.
- **The pad in hand.** With two pads connected (or Steam's virtual pad beside the real one), the
  one used last drives.
- **Focus.** When the game window loses focus (Alt-Tab, a notification), every held button and
  stick is let go, since Windows stops reporting the pad to a window without focus.
- **The keyboard and the mouse** stay live beside the pad. F10 opens the launcher at the login
  screen and the settings in the world; the launcher also opens by itself at the login screen.

Limits:

- The Xbox (Guide) button belongs to Windows' Game Bar, and Windows.Gaming.Input offers neither
  it nor the Share button nor an Elite's paddles. The layout uses none of them.
- Rumble is not wired yet.
- With Steam running, Steam Input may present its own virtual pad: either turn it off for
  Vanillish, or leave it, since the pad in hand still drives.

## Where the files are

Everything is in one folder, `%LOCALAPPDATA%\Vanillish` (paste that into File Explorer's address
bar). *Settings → Debug → Open the Vanillish folder* opens it. The exception is a portable copy,
below.

| path | what |
|---|---|
| `benilla-config\` | benilla's state folder: `config.toml`, key bindings, macros, SavedVariables, `AddOns\`, screenshots |
| `benilla-config\inilla\` | Vanillish's settings, servers and accounts, controller mapping, `install.toml` (your WoW folder), `secrets.bin` |
| `benilla-config\Diagnostics\` | `session-<time>.log` (the last 10) and `crash-<time>.txt` |
| `Data\` | the game data Vanillish downloaded or copied in (not used while your own client is in place) |
| `Cache\` | temporary files |

- **Portable.** Put an empty `portable.txt` beside `Vanillish.exe`, and everything above lives in
  the exe's own folder instead. The exception is a folder that holds `WoW.exe`: there it never
  does.
- **OneDrive.** Nothing goes into Documents or the Desktop, which OneDrive may sync.

## Passwords

A saved password, a server's access code and the admin code are sealed with DPAPI under your
Windows account, in `benilla-config\inilla\secrets.bin`.

- Another Windows user, or another PC with the folder copied over, cannot unseal it. The file is
  set aside as `secrets.bin.unreadable`, and the passwords are asked for again.
- An earlier desktop run's plain `secrets.toml` is sealed in and deleted.

## The log and a crash

- **The session log.** The player build has no console window. Everything the client prints goes
  to `benilla-config\Diagnostics\session-<time>.log`, line by line, and to the in-app log viewer
  (*Settings → Debug → View session log*) and the remote console.
- **`--console`.** `Vanillish.exe --console`, started from a Command Prompt, PowerShell or Windows
  Terminal, prints the same lines there too.
- **Without a console.** With no console attached, the lines also go to the debugger output, which
  Sysinternals' DebugView shows.
- **A crash.** If the game stops, on a panic or with an error code, a message box says so and
  names the folder that holds the crash report. When the renderer stopped it (the GPU device was
  lost), the box says the graphics driver stopped responding. The report's backtrace names
  functions, because `vanillish.pdb` sits beside the exe. The session log's last lines say why the
  app ended (`vanillish: the app is ending`) and when the GPU device was lost.
- **The graphics API.** Vanillish uses Vulkan, else Direct3D 12 (never OpenGL); its log's
  `AdapterInfo` line names the GPU and the API. To try the other one, start it from a Command
  Prompt with `set WGPU_BACKEND=dx12` (or `vulkan`) first. `set WGPU_SETTINGS_PRIO=compatibility`
  asks the GPU for its baseline features only.
- **egui without bindless.** Vanillish's own screens draw without egui's bindless texture arrays on
  Windows: with them, the first screen lost the Direct3D 12 device within seconds, and the game
  ended (measured on Microsoft's WARP; `VANILLISH_EGUI_BINDLESS=1` turns them back on).
- **The remote console** ([DEBUGGING.md](DEBUGGING.md)) lists the PC's LAN address: Wi-Fi and
  Ethernet first, WSL, Hyper-V, Docker and VPN adapters last. The first time, Windows Firewall
  asks whether Vanillish may accept connections: allow it on Private networks.

## Settings that differ

| tab | on Windows |
|---|---|
| General | Only *Modern lighting (restart)* and *Keep the screen on*. The latter is a Windows power request, which also keeps the PC awake during a download. The rest of the tab is the phones': the graphics presets, the render resolution and the interface size are the stock Video options on a PC. |
| Touch | Not shown. |
| Controller | iNilla's, unchanged. |
| Audio | The phones' audio-session rows are not shown; Windows mixes every app. |
| Debug | *Show touch zones* is not shown. *Write a session log* names its folder. *Open the Vanillish folder*, and *Share…* on a log shows it in File Explorer. *Game data…* opens the setup screen. |
| Server | The admin menu ([ANDROID.md](ANDROID.md#the-server-admin-menu)): *Click again* in place of *Tap again*. |

## Where it lives

| path | what |
|---|---|
| `crates/vanillish-windows` | The exe's entry. It picks the Vanillish folder (`inilla_platform::paths::default_root`), then boots and runs the client. It also holds `--console` and the message box on a panic. Its `build.rs` stamps the build and embeds the icon, the manifest and the version. |
| `crates/inilla-platform/src/windows/` | The Win32 and WinRT calls behind the plain functions: the log capture, the DPAPI keychain, power requests, metered networks, the folder dialog, File Explorer, the clipboard, the LAN addresses, free space. |
| `crates/inilla-platform/src/paths.rs` | The Windows folder (`set_root`, `choose_root`, portable mode), and your WoW folder in place (`set_wow_folder`, `data_in_place`). |
| `crates/inilla-core/src/pads.rs` | A pad's glyph family from its USB ids, shared with Android. |
| `crates/benilla-app/src/inilla/` | With the `vanillish` feature: the words (`ui/words.rs`, `WINDOWS`), the controller on by default and the pad in hand (`pad.rs`), the window's title, icon and owner handle (`windows.rs`), the in-place data (`data.rs`). |
| `windows/` | The resources (`vanillish.rc`, `vanillish.manifest`, `Vanillish.ico`, made by `tools/make-icon.py`), `build-exe.sh`, and `toolchain/linux-env.sh` (the Linux cross check). |

benilla-app's `vanillish` feature is the product switch. `vanillish-windows` turns it on, and only
for a Windows target, so:

- upstream's `cargo play` on Windows stays benilla's desktop rehearsal;
- no Linux gate builds the feature.

Some Windows arms of `inilla-platform` are `cfg(windows)` alone and reach that rehearsal too: the
log capture, DPAPI, the folder dialog and File Explorer.

## Known limits

- **Not signed.** SmartScreen asks once per release, and Smart App Control blocks the exe.
- **Pad and Vanillish's own screens.** The pad does not move through the launcher and settings
  screens, which need the mouse or F10. Esc on those screens reaches the game.
- **One cursor.** In cursor mode the virtual cursor and the Windows cursor are both drawn.
- **No rumble.** Impulse triggers are not wired either.
- **Restart.** The new process starts before the old one ends, and the old one's exit savers do not
  run (as on Android). A CVar changed less than a second before **Restart Vanillish now** can be
  lost.
- **A copy is not resumed after a restart.** A copy that Pause or a crash cut short does not offer
  to resume once the game has been restarted; *Import from folder…* starts it again.
- **x86-64 only.** No Arm64 build yet; on Snapdragon PCs the x64 exe runs under emulation.

## What needs a Windows PC

Vanillish for Windows has been built and checked from Linux only: cross type-checks and clippy for
`x86_64-pc-windows-gnu`, and the Linux host tests. Nothing above has run on Windows yet. The first
runs should check:

1. **The `vanillish-windows` workflow.**
   - `rc.exe` compiles `windows/vanillish.rc`;
   - the exe's imports fit `build-exe.sh`'s allow-list (widen it on purpose if a system DLL is
     missing);
   - the Windows unit tests pass, the DPAPI ones included.
2. **First start from Explorer.**
   - SmartScreen, then a full-screen window titled Vanillish with the V icon in the title bar and
     the taskbar;
   - Connect is the first screen;
   - `session-*.log` fills line by line;
   - `--console` prints into cmd, PowerShell and Windows Terminal, and `> out.txt` writes the file.
3. **Connect.**
   - The download goes into `…\Vanillish\Data`;
   - on metered and unmetered Wi-Fi it waits for the button or starts by itself;
   - **Restart Vanillish now** comes back and logs in by itself.
4. **Use my WoW folder.**
   - A folder with `WoW.exe` and a bare `Data` folder are both accepted, and an incomplete one is
     refused;
   - after the restart the game boots from it;
   - nothing in that folder changes: compare a listing with sizes and times before and after a
     session.
5. **Controllers.**
   - Xbox over USB, over Bluetooth and through the adapter: glyphs, hot-plug mid-session, the
     trigger layers, cursor mode, the radial;
   - a DualSense gets PlayStation glyphs (log the name Windows reports, and its ids);
   - with two pads, the one in hand drives;
   - Alt-Tab while walking lets go of the movement;
   - with Steam running, Steam Input on and off.
6. **The folder dialog.** It opens in front of the full-screen game and modal to it, and Cancel
   gives nothing.
7. **Passwords.**
   - They survive a restart;
   - `secrets.bin` under another Windows user is set aside and asked for again.
8. **The rest.**
   - `powercfg /requests` shows Vanillish while in the world or downloading;
   - the remote console's address is the LAN one;
   - the setup screen's free space is right;
   - *Open the Vanillish folder* and the log's *Share…* open File Explorer;
   - the message box after a forced panic (`WOW_CRASH_INJECT` in a dev build).
