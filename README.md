<div align="center">
  <h1>Vanillish</h1>
  <p><b>iNilla — benilla, the from-scratch World of Warcraft 1.12.1 client in Rust — on Android, made for the Galaxy Z Fold8 Ultra</b></p>
</div>

**Vanillish** is [iNilla](https://github.com/Martze96/iNilla) built for Android: the same client,
the same touch controls, controller mapping, launcher and remote console, with what an Android
phone needs around them. It is sideloaded as an APK, for Android 12 or later on an arm64 phone with
Vulkan 1.1.

**The simplest way to play.** On a Mac with Apple silicon, with Docker Desktop or OrbStack
installed, run one command in Terminal to start your own server:

```sh
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/klanthier/vanillish/klanthier/inilla-android-port/server/setup-mac.sh)"
```

It asks for your World of Warcraft 1.12.1 folder, starts a 1.12.1 realm in Docker (nothing is
compiled), and ends with the address to type. Then [install Vanillish](#download--install), open
it, type that address under **Server address** and tap **Connect**: the game files come from your
Mac over Wi-Fi (about 5 GB, once), **Create account** makes your account, and after **Restart
Vanillish now** it logs in by itself. Linux and Windows (WSL) have the same in `setup-linux.sh`
([server/README.md](server/README.md)).

Vanillish is tuned for the **Samsung Galaxy Z Fold8 Ultra**:

- **The cover screen** plays iNilla's phone layout as it is — a 21:9 screen about an iPhone Pro
  Max's size, with the camera hole where the Dynamic Island would be.
- **The inner screen** has a layout of its own for two hands on the side edges: the stick under
  the left thumb, jump and twelve action slots around the right one, and an interface sized so the
  main action bar still fits. Fold or unfold and the right layout comes up by itself; each screen
  keeps its own edits.
- **Tabletop (Flex mode)**: half fold the phone and stand it on a table, and the world and the
  game's windows move above the hinge while the half below becomes a control deck — the stick, the
  action buttons, a pad to look around.
- **120 Hz**: the graphics presets ask the panel for 30, 60 or its full 120 Hz, and Game
  Dashboard's Performance and Battery modes pick the preset.
- **Samsung DeX, a mouse, a keyboard**: the mouse works as on the desktop — five buttons, the
  wheel, mouse-look with the pointer captured, the game's own cursors — and the touch controls step
  aside while it is in use.
- **Controllers**: Xbox, DualSense, DualShock 4, Switch Pro, the Backbone One and other USB or
  Bluetooth pads, in iNilla's WoW Forever layout with the pad's own button symbols; a USB-C clip on
  the folded phone makes the cover screen a handheld.
- **Staying connected**: a look at another app or folding the phone shut does not drop you from
  the server; half an hour after your last input the character logs out, as the game's own idle
  logout would.
- And Android's own: the system keyboard with its suggestions, Back as the game's Escape, passwords
  sealed with an Android Keystore key, logcat and the share sheet for logs.

Other arm64 phones and tablets get the phone or the inner-screen layout by their screen's shape.
Vanillish has been built and checked on Linux only so far; [docs/ANDROID.md](docs/ANDROID.md#what-needs-a-device)
lists what the first device runs must confirm.

## Download & install

1. **Get the APK**:
   - a release, `Vanillish-<version>.apk`, from [**Releases**](https://github.com/klanthier/vanillish/releases)
     (the `vanillish-v*` tags); or
   - a test build: [**Actions → vanillish-apk**](https://github.com/klanthier/vanillish/actions/workflows/vanillish-apk.yml)
     → a run → its artifact `Vanillish-<commit>.apk` (the APK itself, not a zip; kept 14 days).
2. **Sideload it**: open the APK on the phone. Android asks once to let the browser (or My Files)
   install unknown apps: allow it. If Play Protect warns about an unknown app: *More details →
   Install anyway*. From a computer: `adb install -r Vanillish-….apk`.
3. **Updates: install the new APK over the old one — never uninstall first.** Uninstalling deletes
   Vanillish's folder: the game data, the settings, the SavedVariables. And Android installs an
   update only when it is signed with the same key as the installed app: releases carry the
   Vanillish release key, while a build made without it (a fork with no signing secrets, a local
   build with no key set) is signed with a debug key and belongs on a phone without Vanillish
   ([docs/ANDROID-BUILD.md](docs/ANDROID-BUILD.md#signing)).
4. **Game data (once)**: the app contains no Blizzard files; the 1.12.1 game files come from your
   own client. The first screen, **Connect**, takes them from your server: type its address, and
   they download over Wi-Fi with your account made alongside
   ([docs/ANDROID.md](docs/ANDROID.md#quick-connect)). Without such a server, **Other ways to get
   the game files…** leads to copying your client's `Data` folder in from a PC over USB (into
   Internal storage → Android → data → io.github.klanthier.vanillish → files), or to **Import from
   folder…** for a copy already on the phone or a USB-C stick. See
   [docs/ANDROID-DATA.md](docs/ANDROID-DATA.md).

The whole APK builds on Linux or macOS, and on CI on a single Linux runner — see
[docs/ANDROID-BUILD.md](docs/ANDROID-BUILD.md).

| read | for |
|---|---|
| [docs/ANDROID.md](docs/ANDROID.md) | what Vanillish adds on Android: quick connect, the Fold8 Ultra's screens and tabletop, input, the frame rate, the settings that differ, where the files are, known limits |
| [docs/ANDROID-DATA.md](docs/ANDROID-DATA.md) | getting the game data onto the phone |
| [server/README.md](server/README.md) | your own server: one command on a Mac, Linux or Windows |
| [docs/ANDROID-BUILD.md](docs/ANDROID-BUILD.md) | building, signing and installing the APK, CI, debugging on Android |
| [docs/INILLA.md](docs/INILLA.md) | the touch controls, the controller, the launcher and the settings, which Vanillish shares |
| [docs/DEBUGGING.md](docs/DEBUGGING.md) | the remote console, logs, the desktop rehearsal |
| [docs/UPSTREAM.md](docs/UPSTREAM.md) | syncing iNilla and benilla, and every file of theirs Vanillish changes |

Servers and clients are iNilla's: a 1.12.1 core benilla supports (vmangos, cMaNGOS) with **Warden
disabled** — [`server/`](server/README.md) is one, set up in one command — and an English (enUS)
1.12.1 client, build 5875. Vanillish is an independent fan
project, not affiliated with or endorsed by Blizzard Entertainment.

---

*Everything below is iNilla's README, unchanged (with upstream benilla's below it).*

<div align="center">
  <h1>iNilla</h1>
  <p><b>benilla — the from-scratch World of Warcraft 1.12.1 client in Rust — on the iPhone iOS 17.0+</b></p>
  <img src="ios/icons/AppIcon1024.png" width="120" alt="iNilla icon">
</div>

**iNilla** is an iOS build of [benilla](https://github.com/samwhosung/benilla), made to be
sideloaded with **SideStore** (or AltStore / LiveContainer), with everything a phone needs around
the unchanged game:

- **Touch controls** that cost no screen space until a thumb lands — a floating movement stick,
  jump with a ring of action slots around it (with real icons and cooldowns), switchable
  **views** (Combat / Explore / Social / Minimal), a radial menu for every window, drag-to-look,
  pinch-zoom, and the stock interface usable by finger (tap = click, long press = right click,
  drag = move items, two fingers = scroll). A **layout editor** moves, resizes, rebinds and hides
  every element.
- **Controllers** — Backbone One, Xbox, DualSense, any MFi pad — in WoW Forever's layout: sticks
  move and look, LB/RB cycle targets, the triggers switch the face buttons and d-pad between four
  layers (32 bindings), a radial menu, a virtual cursor that opens by itself with loot, vendor and
  quest windows. Fully remappable.
- **A launcher**: server profiles (LAN or internet, with a realm-address override for NAT),
  accounts with passwords in the iOS Keychain, one-tap login and account switching, auto-login.
- **Game data, once**: copy your own `Data` folder in with Finder / the Files app, or download
  it directly from a server with HTTP data hosting (resumable, SHA-256 verified). The companion
  [iNilla server](https://github.com/Martze96/iNilla-server) is an optional helper repository (basically standard
  vmangos bundled with a game-data HTTP endpoint) to easily get a server with data hosting running in Docker. The app contains no Blizzard files.
- **A fast debug loop without a Mac**: a remote console in any browser on the same Wi-Fi (live
  log, Lua, commands, screenshots, config files), session logs and crash reports in the Files app,
  and a desktop build that rehearses the touch HUD and controller.

<p align="center">
  <img src="docs/images/touch-hud.jpg" width="49%" alt="iNilla Touch HUD">
  <img src="docs/images/radial-menu.jpg" width="49%" alt="Radial Menu">
</p>
<p align="center">
  <img src="docs/images/gesture-hub.jpg" width="49%" alt="Gesture Hub on Jump">
  <img src="docs/images/settings.jpg" width="49%" alt="In-game Settings & Overlays">
</p>

## Download & Install

1. **Download the `.ipa`**: Grab the latest release package (`iNilla-*.ipa`) from [**Releases**](https://github.com/Martze96/iNilla/releases).
2. **Sideload (iOS 17.0+)**:
   - **SideStore / AltStore**: In SideStore/AltStore on your phone, go to **My Apps** → **＋** and select the `.ipa`. It will be signed with your Apple ID.
   - **LiveContainer**: You can also run iNilla directly as a LiveContainer guest without consuming an app ID slot.
3. **Game Data (One-Time Setup)**:
   - On first launch, the app prompts for the 1.12.1 game files (the app contains no Blizzard files).
   - **Easiest:** Download them over Wi-Fi from your [**iNilla server**](https://github.com/Martze96/iNilla-server) (automated, resumable, and SHA-256 verified).
   - **Alternative:** Copy your existing `Data` folder directly into the app using Finder or the iOS Files app (see [docs/DATA.md](docs/DATA.md)).

Everything is built on Linux except the final link, which is a few minutes on a macOS runner —
see [docs/IOS-BUILD.md](docs/IOS-BUILD.md).

| read | for |
|---|---|
| [docs/INILLA.md](docs/INILLA.md) | what iNilla is made of, the controls, the settings |
| [docs/IOS-BUILD.md](docs/IOS-BUILD.md) | building and installing the `.ipa` (SideStore, LiveContainer), CI and its minutes |
| [docs/DATA.md](docs/DATA.md) | getting the game data onto the phone; the data host |
| [docs/DEBUGGING.md](docs/DEBUGGING.md) | the remote console, logs, crash reports, the desktop rehearsal |
| [docs/UPSTREAM.md](docs/UPSTREAM.md) | syncing benilla, and every upstream file iNilla touches |

Servers: any 1.12.1 core benilla supports — **vmangos** (what benilla develops against) and
**cMaNGOS** — with **Warden disabled** (benilla does not implement it). (Tip: use [iNilla server](https://github.com/Martze96/iNilla-server)
if you want a ready-to-run vmangos instance that already serves game data to the app). Clients: English (enUS)
1.12.1, build 5875. iNilla is an independent fan project, not affiliated with or endorsed by
Blizzard Entertainment.

---

*Everything below is upstream benilla's README, unchanged.*

<div align="center">
  <h1>benilla</h1>
  <p><b>A complete World of Warcraft 1.12.1 client, written from scratch in Rust and <a href="https://bevy.org">Bevy</a></b></p>
  <p>
    <a href="https://discord.gg/wJSJx467G4"><img src="https://img.shields.io/discord/1529280129518538922?style=for-the-badge&logo=discord&logoColor=white&label=discord&color=5865F2" alt="Discord"></a>
    <a href="https://www.youtube.com/playlist?list=PLdCnpZNKxyb8"><img src="https://img.shields.io/badge/devlog-youtube-FF0000?style=for-the-badge&logo=youtube&logoColor=white" alt="YouTube devlog"></a>
    <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue?style=for-the-badge" alt="License"></a>
  </p>
</div>

benilla plays the whole game: character creation, questing and professions, dungeons and raids,
battlegrounds and honor, groups, guilds, trade, mail and the auction house, on the stock 1.12
interface and with your addons. It connects to a 1.12.1 server over the original protocol and
reads the game's data from your own 1.12.1 install. Every file format, the network protocol and
the interface engine are written from scratch, with no original client code, no third-party WoW
crates and no bundled game assets.

## What's inside

- **Formats:** readers for the whole asset stack (the MPQ patch chain, BLP, DBC, ADT, WDT, WDL, M2
  and WMO), wired into Bevy as an asset source.
- **World:** terrain streamed out to the horizon, portal-culled buildings with interior lighting,
  doodads and ground clutter, swimmable water, sky and weather, and the client's own day/night
  lighting, fog and gamma.
- **Models:** GPU-skinned M2s with the full animation controller, particles, ribbons and the spell
  visuals, and characters end to end: customization, the armor composite, weapons with their
  enchant glows, forms, stealth and mounts.
- **Movement:** networked movement in both directions, the server-granted modes from slow fall to
  roots, a follow camera with collision, boats, zeppelins and taxi flights.
- **Networking:** SRP6 auth through world-session crypto and the object mirror into the ECS,
  covering the game from movement and chat through combat, spells, groups and raids, quests,
  trade, mail, the auction house and battlegrounds.
- **Interface:** a FrameXML and Lua engine that runs the stock 1.12 interface off your install's
  patch chain, and third-party 1.12 addons: Questie, pfUI, Bagnon, Bartender2 and most others
  run. By choice, the options window and the ESC menu follow the Classic Era client's rather than
  1.12's.
- **Audio:** music, ambience and effects under the client's own selection and crossfade rules,
  with interior and underwater transitions and zone reverb.

The format readers and the UI engine core are plain Rust with no Bevy in them, and the world
renderer runs with no game attached. [`docs/MAP.md`](docs/MAP.md) maps every crate and subsystem,
generated from the code.

## Status

Complete and fully playable. What is left:

- The long tail of small features that separates a working client from a finished one, tracked
  as [issues](https://github.com/samwhosung/benilla/issues).
- Addons, options and performance, ongoing.

benilla is a faithful 1.12.1 client and the foundation people build on. There are no prebuilt
downloads: a packaged build, a particular server's changes or anything 1.12.1 never had belongs
in a fork, and forks are welcome. GitHub lists
[every public fork](https://github.com/samwhosung/benilla/forks).

Not planned: other expansions or client versions, Warden (anticheat).

## Running it

benilla builds and runs on macOS, Linux and Windows. You need:

- **An English 1.12.1 client (build 5875)** for the game data. benilla only reads it.
- **A 1.12.1 server with Warden off.** [vmangos](https://github.com/vmangos/core) is what
  development runs against, and it ships with Warden off; cMaNGOS and the other 1.12.1 cores speak
  the same protocol.
- **Stable Rust and a C compiler**, because the client's Lua is built from source: on macOS the
  Xcode command line tools, on Linux the ALSA and udev development packages and pkg-config, on
  Windows the MSVC build tools that the Rust installer sets up.

```sh
WOW_DATA=/path/to/WoW/Data cargo run --release -p benilla
```

On Windows, in PowerShell:

```powershell
$env:WOW_DATA="C:\path\to\WoW\Data"; cargo run --release -p benilla
```

Each release is a tag on the [Releases page](https://github.com/samwhosung/benilla/releases):
`git checkout <tag>` first runs that release, and `main` is the development tip.

`WOW_DATA` names the install's `Data` folder; a link to the install named `WoW` at the repo root
does the same (`ln -s /path/to/WoW WoW`, or on Windows a junction, which needs no admin rights:
`New-Item -ItemType Junction -Path WoW -Target C:\path\to\WoW`). The server defaults to
`localhost:3724`, the stock auth port. Point `WOW_HOST` at another (`WOW_HOST=play.example.com`, or
`play.example.com:5000` for a remapped port), or set it from the Realmlist button on the login
screen, which remembers it. Credentials go in at the login screen, or set `WOW_USER` and `WOW_PASS`
to skip it.

Settings, screenshots and addons live in `benilla-config/` at the repo root: a 1.12 addon goes in
`benilla-config/AddOns/`. [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) has the rest, from the
player build to the tests.

## Contributing

Issues and pull requests are open. [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) says where to
start, what gets in, how a change is judged and what happens to a pull request once it is open.
Bugs, questions and ideas are welcome on the [Discord](https://discord.gg/wJSJx467G4) too.

---

Early inspiration and file format guidance came from the
[wowemulation-dev](https://github.com/wowemulation-dev) community, and
[warcraft-rs](https://github.com/wowemulation-dev/warcraft-rs) in particular.

benilla is an independent fan project, not affiliated with or endorsed by Blizzard Entertainment.
It ships **no Blizzard content**: no art, models, sounds, maps, MPQ contents or FrameXML. You
provide your own legally obtained 1.12.1 client, and the stock interface runs off its FrameXML at
runtime. The few files under `crates/benilla-app/assets/ui/` are our own, not copies of it:
adapters over stock files, and the settings windows and script error log benilla draws itself.

World of Warcraft is a trademark of Blizzard Entertainment, Inc. Our own code is licensed under
[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option. The two vendored components
under `third_party/`, the kira audio engine and a Lua 5.1 patched to the 1.12 client's dialect,
keep their own upstream licenses, alongside each.
