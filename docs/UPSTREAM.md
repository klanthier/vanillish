# Vanillish and upstream iNilla / benilla

Vanillish is a fork of [Martze96/iNilla](https://github.com/Martze96/iNilla), which is itself a fork
of [samwhosung/benilla](https://github.com/samwhosung/benilla) that merges upstream as it
publishes (below). Vanillish merges both the same way: real merge commits, never a rebase or a
squashed copy. Its own code is in its own places — `crates/vanillish-android`, `android/`,
`crates/inilla-platform/src/android/`, the `vanillish-*` workflows — and in new files inside
iNilla's crates; changes inside iNilla's or benilla's files are carries (listed below), kept few
and small so a merge conflicts on as little as possible. Its server, `server/`, is iNilla-server's
taken in as files, and synced by its own diff ([below](#server-and-inilla-server)).

## Syncing

The remotes are `inilla` and `upstream` (on a fresh clone, add them once):

```sh
git remote add inilla https://github.com/Martze96/iNilla        # once
git remote add upstream https://github.com/samwhosung/benilla   # once
git fetch inilla upstream
git checkout -b sync-inilla-$(date +%Y%m%d)
git merge inilla/main            # iNilla, with the benilla it has merged
git merge upstream/main          # only when benilla has moved on and iNilla has not taken it yet
# resolve the carries below if they conflict, then:
cargo test -p inilla-core -p inilla-platform
cargo check -p benilla-app --no-default-features
cargo ndk -t arm64-v8a -P 31 check -p vanillish-android
python3 android/tools/test_jni_natives.py
(cd android && ./gradlew --no-daemon -q :app:compileReleaseJavaWithJavac)
```

and, where the iOS toolchain is set up, iNilla's own check
(`source ios/toolchain/linux-env.sh && cargo check --target aarch64-apple-ios -p inilla-ios`):
Vanillish keeps iOS building, so it can go back to iNilla. Open a PR from the sync branch and merge
it **with a merge commit**.

**iNilla first.** iNilla's sync of benilla has already resolved iNilla's own carries; merge benilla
directly only for what iNilla has not taken yet. A benilla change merged here first comes back
later inside iNilla's merge, with iNilla's resolution of its carries, which may differ from the one
made here — resolve that conflict iNilla's way.

What a sync can break without a conflict, and the build alone will not show:

- **New platform code in iNilla.** A new `#[cfg(target_os = "ios")]` / `#[cfg(not(target_os =
  "ios"))]` pair, or a `cfg!(target_os = "ios")`, puts Android on the desktop's arm. It compiles,
  and does the desktop's thing on the phone. Right after each merge, look at every one it brought:
  `git diff ORIG_HEAD HEAD -- crates/inilla-core crates/inilla-platform crates/benilla-app/src/inilla | grep -n 'target_os'`.
- **New words.** A sentence that names the iPhone, the Files app or the Keychain belongs in
  `crates/benilla-app/src/inilla/ui/words.rs`, with an Android form.
- **A new Java-facing contract.** A Java `native` without its function in `jni_exports.rs` fails
  `test_jni_natives.py`; a Java class Rust calls by name needs a `-keep` in
  `android/app/proguard-rules.pro`, or R8 strips it from the release APK only.
- **android-activity.** When Bevy moves to another android-activity, `build-rust.sh` stops until
  `vanillish.androidActivity` and `vanillish.gamesActivity` in `android/gradle.properties` are
  updated together ([ANDROID-BUILD.md](ANDROID-BUILD.md#toolchain)).
- **A new dependency with no Android backend** (as `arboard` has none): the Android type-check
  catches it; gate it like `arboard` in `crates/benilla-app/Cargo.toml`.
- **A lighting change in a world shader.** `terrain.wgsl`, `wow_model.wgsl`, `liquid.wgsl` and
  `static_gx.wgsl` carry the modern lighting path behind `#ifdef VANILLISH_MODERN`, which reads
  the stock code around it. A benilla change there merges cleanly and the stock look stays right,
  but the modern path may need the same change; `cargo test -p benilla-assets --lib compose_tests`
  composes every shader with and without the look, and the eye judges the rest
  (`docs/ANDROID.md`, "Modern lighting").

## The carries

Every file of benilla's and of iNilla's that Vanillish changes, and why. In benilla's files each
change is marked `Vanillish:` in a comment (`Cargo.lock` aside: it is generated); in iNilla's, each
change that adds Android behaviour is.

**In upstream benilla's files:**

| file | change |
|---|---|
| `Cargo.toml` | `[profile.android]` (release, no LTO, symbols kept), beside iNilla's `[profile.ios]` |
| `Cargo.lock` | the `vanillish-android` package; `jni` and `ndk-context` for `inilla-platform`. After a conflict, take the incoming one and let a `cargo check` add these back, rather than merge it by hand |
| `crates/benilla-app/Cargo.toml` | `arboard` not on Android either: it has no backend there (the clipboard is `inilla-platform`'s); `benilla-srp` in `[dependencies]`, for quick connect's sign-up, which makes the account's SRP6 salt and verifier on the phone (already in the tree through `benilla-protocol`) |
| `crates/benilla-app/src/textinput/clipboard.rs` | iNilla's iOS pasteboard backend serves Android too (`ClipboardManager`, through `inilla-platform`) |
| `crates/benilla-app/src/world_backdrop.rs` | `WorldViewport`: the world drawn in part of the window — above the hinge on a tabletop — with the size-carrier and the world camera following it; `world_frame_floor`, the bottom the `WorldFrame`-anchored overlays measure up from (the window's, which on a tabletop is not the world's); tests |
| `crates/benilla-app/src/vplates.rs` | nameplates seated up from `world_frame_floor` instead of the world viewport's height: on a tabletop every plate sat a deck-height low, on the deck below the hinge |
| `crates/benilla-app/src/chat_bubble.rs` | chat bubbles seated up from `world_frame_floor`, as the plates; the bubble of a speaker behind the camera at the world's bottom-left corner (the hinge's left end on a tabletop), not the window's |
| `crates/benilla-app/src/target/hover.rs` | the unit, object and occlusion hovers trace nothing for a cursor off the drawn world (`cursor_over_view`): over a tabletop's deck nothing in the world is hovered |
| `crates/benilla-app/src/player/camera.rs` | `cursor_in_viewport` asks `cursor_over_view`: on a tabletop the world camera draws into a target the size of the half above the hinge, with no `viewport`, so a press on the deck below counted as a world press (mouse-look) |
| `crates/benilla-app/src/net/io.rs` | off the screen on Android no update runs, so the write thread runs the idle handler's camp (`set_off_screen`, `off_screen_step`, tested): `CMSG_LOGOUT_REQUEST` thirty minutes after the last input, the socket shut five minutes later if the character is still in the world; the read thread stops the stay-connected service when the world session ends or the link is lost |
| `crates/benilla-app/src/ui_chat/idle.rs` | `AUTO_LOGOUT_AFTER` and `LastInput::at` `pub(crate)`, for that camp |
| `crates/benilla-protocol/src/world/writer/lifecycle.rs` | `WorldWriter::shutdown`: the socket shut both ways, so the read half ends as on a dropped link — the camp's end when the logout was refused (in combat) |
| `crates/benilla-world/src/ffx_glow.rs` | `FfxBackdrop.rect`: the world's combine drawn into that rect, then the interface over the whole target again |
| `crates/benilla-world/src/boot.rs` | `unpipelined_on_android`: no pipelined rendering on Android, where a surface taken mid-present is lost and Bevy panics; `VANILLISH_PIPELINED=1` keeps it, to measure |
| `crates/benilla-world/src/interact/pick.rs` | `cursor_over_view`: a cursor outside what the camera draws (a tabletop's deck) casts no ray, as `viewport_to_world` would cast one below the drawn frustum; `pick_at_cursor` and `pick_object_at_cursor` apply it; a camera whose target is not measured yet refuses nothing |
| `crates/benilla-world/src/interact/mod.rs` | `pub use pick::cursor_over_view`, for the app's other cursor rays (the hovers, the camera) |
| `crates/benilla-assets/Cargo.toml` | dev-dependencies `naga` and `naga_oil`, for the world shaders' composition tests (`modern_light::compose_tests`); both already in the graph through Bevy, with no feature added |
| `crates/benilla-assets/src/lib.rs` | `mod modern_light` and its exports: the opt-in modern lighting look, boot-latched like the texture filter |
| `crates/benilla-assets/src/materials.rs` | `register_shaders` loads `vanillish_modern.wgsl` as an import library (embeds it only where there is no `Shader` asset); the terrain, model, WDL and liquid materials' `specialize` (new on terrain and WDL) push `modern_lighting()`'s defs |
| `crates/benilla-assets/src/shaders/terrain.wgsl`, `wow_model.wgsl`, `liquid.wgsl`, `crates/benilla-world/src/shaders/static_gx.wgsl` | the modern lighting path, every line behind `#ifdef VANILLISH_MODERN` (with the def off, each file is today's token for token): the sun and the three picked point lights per pixel on a relief normal from four albedo taps, the cavity, the sheen; on water the sheen per pixel and the fresnel sky lift. A benilla change to these files' lighting may need its twin in the modern path: the composition tests prove only that it still compiles |
| `crates/benilla-world/src/static_gx/render.rs` | the four pipelines take `modern_lighting()`'s defs |
| `crates/benilla-world/src/worldview.rs` | logs which lighting look the run renders (`$VANILLISH_MODERN`), so a before/after pair proves its variant |
| `crates/benilla-app/src/lib.rs`, `crates/benilla-app/src/game_plugins.rs` | `mod modern_lighting` and its plugin (the `modernLighting` knob and boot latch), before `CvarPlugin` |
| `crates/benilla-app/src/cvars/table.rs` | the `modernLighting` row, `ours`, default `"0"` (the reference's look) |
| `crates/benilla-app/src/cvars/mod.rs` | `$VANILLISH_MODERN` owns `modernLighting` for the session; the tests' knob, observer and arm |
| `crates/benilla-app/src/pipe_warm/mod.rs` | a paragraph: the modern look is one def set latched at boot, so the warm set needs no axis for it |

**In iNilla's files:**

| file | change |
|---|---|
| `README.md`, `AGENTS.md`, `docs/UPSTREAM.md` | a Vanillish section on top; iNilla's text below it unchanged |
| `.github/workflows/inilla-ipa.yml` | tags `v*` but not `vanillish-v*`, which `vanillish-apk.yml` builds |
| `crates/inilla-core/src/lib.rs` | `mod fold`, `mod import`, `mod connect` |
| `crates/inilla-core/src/settings.rs` | `tabletop_split`, `inner_any_orientation`, `stay_connected`, `last_game_mode`, `hide_touch_with_mouse`, `modern_lighting` (and `modern_lighting_cvar`, the CVar it mirrors); `GameMode` and `follow_game_mode` (Game Dashboard picks the preset), tested |
| `crates/inilla-core/src/touch.rs` | the screen split at a hinge (the world above, the deck below) and the deck's look pad (`park`, public for the controller's look); pen hover, with no cursor over the deck; mouse buttons 3–5; the layout's screen class; the hub's radii; on the inner screen and the tabletop the radial at the thumb and capped at 170 pt (a phone class, so every iOS device, keeps iNilla's size) |
| `crates/inilla-core/src/data.rs` | the build-mismatch message says "this client", not "iNilla" |
| `crates/inilla-core/src/profiles.rs` | `ServerProfile::info` (where the server described itself, for quick connect to ask again), `DataSource::locale` (the language picked, so a resumed download stays in it), `DataSource::token_key` (the access code's keychain key); both fields default, so an older `servers.toml` loads (tested) |
| `crates/inilla-platform/Cargo.toml` | `jni` 0.21 and `ndk-context` on Android |
| `crates/inilla-platform/src/lib.rs` | `mod android`, `mod picker`, `mod pointer` |
| `crates/inilla-platform/src/device.rs` | an Android arm for every call; new calls (no-ops on iOS): `fold`, `fold_in_window` (`$INILLA_FOLD` on a desktop), `set_frame_rate`, `refresh_rate`, `game_mode`, `take_back_presses`, `set_session_service`, `set_orientation`, `set_gesture_exclusion`, `relaunch`, `quit`, `thermal_headroom`, `network_metered` (`$INILLA_METERED` on a desktop) |
| `crates/inilla-platform/src/paths.rs` | the Android roots (`Android/data/<package>/files`, `.vanillish/Data`), `Host::Android`, `android_package`; `storage_unavailable`: shared storage did not answer at launch (asked again for about three seconds), so the roots are a private stand-in |
| `crates/inilla-platform/src/keychain.rs` | Android: sealed with an Android Keystore key, in private storage; `Unsealed` and `judge` (tested): only a key gone for good forgets the file, a busy Keystore keeps it |
| `crates/inilla-platform/src/pasteboard.rs` | Android's `ClipboardManager` |
| `crates/inilla-platform/src/audio.rs` | Android: audio focus for "mix with other apps" |
| `crates/inilla-platform/src/gamepad.rs` | `NATIVE` (the OS reports the pads: iOS and Android); `mod android` |
| `crates/inilla-platform/src/stdio.rs` | Android: the captured output goes to logcat (tracing's own lines excepted), each line's head cut on a char boundary; everywhere, fds 1 and 2 are pointed back at the original stderr (or `/dev/null`) when the reader thread ends, so a later print never meets a dead pipe |
| `crates/benilla-app/src/inilla/mod.rs` | the `android`, `screen`, `host_pointer`, `connect` modules (`ConnectPlugin`); `touch_device()` true on Android; `Inilla::other_layouts` |
| `crates/benilla-app/src/inilla/boot.rs` | Android's log filter, `TMPDIR`, `RUST_BACKTRACE`; off iOS, the launch-time import only as a rename of a settled, whole folder (`import::launch_guard`) |
| `crates/benilla-app/src/inilla/console.rs` | the addresses ranked for Android's interface names too: Wi-Fi, then the hotspot and USB tethering, then anything else in the system's order, a VPN last; cellular data and its 464XLAT address (`v4-*`) dropped; the page under the app's name |
| `crates/benilla-app/src/inilla/data.rs` | the folder import job, which refuses a picked folder whose files changed within `SETTLE`; Check for copied data's settled and cut-short guards off iOS; their messages; `needs_restart` also when a job added, removed or replaced an archive since the boot; no job but the manifest's while `paths::storage_unavailable`; quick connect borrows `Http` (`Http::quick`, short timeouts; the sign-up's JSON `POST` as `connect::Post`), `SPARE` and `TOKEN_REFUSED` |
| `crates/benilla-app/src/inilla/hud.rs` | `HudScreen`; the tabletop's crease and deck; the chat bar's composing word; the controller radial above the hinge |
| `crates/benilla-app/src/inilla/icons.rs` | button tooltips anchored over the world's rect (above the hinge on a tabletop), offset by `UIParent`'s edges, in its own units |
| `crates/benilla-app/src/inilla/mouse.rs` | the middle, back and forward buttons |
| `crates/benilla-app/src/inilla/pad.rs` | `gamepad::NATIVE` in place of `cfg!(target_os = "ios")`; on a tabletop the cursor kept to the world above the hinge, and the look pressed where the touch look parks |
| `crates/benilla-app/src/inilla/platform.rs` | `ImePurpose`, `BackPresses`; Android's safe area, re-read after a screen change; the screen kept on during a data job; iNilla's own fields raise the keyboard; the interface scale per screen class; the preset's CVars only when the preset changes; the modern lighting row kept in step with `modernLighting` both ways, on a desktop too (off iOS); `Pacing` (the frame cap stands down when the display paces); `clear_the_island` for a top cut-out and the hinge, and while it insets, the stock open-bag stack (`updateContainerFrameAnchors`) measuring `UIParent` instead of the screen (`island_chunk`, tested) |
| `crates/benilla-app/src/inilla/store.rs` | a layout file per screen class; `remove_layout` |
| `crates/benilla-app/src/inilla/touch.rs` | `HudScreen`; the HUD steps aside for a hardware mouse or keyboard; pen hover |
| `crates/benilla-app/src/inilla/ui/mod.rs` | the words from `words.rs`; `restart()` relaunches; Back closes iNilla's screens first; each field's `ImePurpose`; the layout editor handed the layout's own screen (`HudScreen.layout_screen`); `Panel::Connect` and `Overlay::{connect, connect_home}`: with quick connect the first screen is Connect (`first_screen`), the setup screen's header leads back to it and so does Back; the font test reads the connect files |
| `crates/benilla-app/src/inilla/ui/settings.rs` | the Android rows, Modern lighting (restart) off iOS, the frame rates, the Touch tab's screen class and tabletop switch, no silent-switch row, Share session log, Reset all resets every screen's layout |
| `crates/benilla-app/src/inilla/ui/setup.rs` | Import from folder…, Resume, the free space, Pause for the copy; the notice in place of the setup when shared storage did not answer at launch; the words; the running job's progress split out (`progress`), for Connect's download |
| `crates/benilla-app/src/inilla/ui/launcher.rs` | the words; password and address fields; with quick connect, **Connect to a server…** for **+ Add server** and **Create account** on a server that described itself; `ServerForm::{prefilled, edit_noted}` (the form Connect opens on a plain realm, with its note), an edit keeping `info` and the data's `locale`; deleting a server deletes its access code |
| `crates/benilla-app/src/inilla/ui/logs.rs` | Android's copy capped to the last lines; Share… |
| `crates/benilla-app/src/inilla/ui/editor.rs` | laid out on the screen it is handed in place of the window's safe area (on a tabletop split at the hinge, the elements on the deck, at the login and character screens too); the deck outlined on a tabletop; a reset puts back this screen's own layout |

Vanillish's new files in iNilla's crates conflict with nothing unless iNilla adds one of the same
name: `inilla-core/src/{fold,import,connect}.rs`, `inilla-platform/src/{picker,pointer}.rs`,
`inilla-platform/src/gamepad/android.rs`, `inilla-platform/src/android/`, and in
`benilla-app/src/inilla/`: `android.rs`, `screen.rs`, `host_pointer.rs`, `connect.rs`,
`ui/connect.rs`, `ui/words.rs`. In benilla's crates, the modern lighting look's:
`benilla-assets/src/modern_light.rs`, `benilla-assets/src/modern_light/compose_tests.rs`,
`benilla-assets/src/shaders/vanillish_modern.wgsl` and `benilla-app/src/modern_lighting.rs`.

Neither benilla nor iNilla builds for Android, so nothing there notices a change that breaks the
Android build; `vanillish-check`'s Android type-check is what catches it.

## `server/` and iNilla-server

`server/` is [Martze96/iNilla-server](https://github.com/Martze96/iNilla-server) at `990cc15`,
taken in as files by `db4c82e8` (its tree is exactly `990cc15`'s), not as a history: there is no
remote and no subtree for it. Its changes since are Vanillish's, below. To take what iNilla-server
has published since, apply its diff under `server/`, on a branch:

```sh
git fetch https://github.com/Martze96/iNilla-server main
git log --oneline 990cc15..FETCH_HEAD        # what is new there
git checkout -b sync-server-$(date +%Y%m%d)
git diff --binary 990cc15 FETCH_HEAD | git apply -3 --directory=server
git status                                   # U: both sides changed it; resolve it Vanillish's way
bash server/ci/setup-test.sh
cargo test --manifest-path server/datahost/Cargo.toml
server/ci/check.sh                           # Docker: the stack with no game data, sign-up, a logon
```

then name the new base commit here in place of `990cc15`. The fetch brings `990cc15`'s objects,
which `-3` needs for its three-way merge. What a sync must keep:

- **The names**: compose project `vanillish`, the images `ghcr.io/klanthier/vanillish-vmangos` and
  `vanillish-datahost`, the volume `vanillish-wowdata`, the user `vanillish_signup`. iNilla-server
  says `inilla` in the same places.
- **The tags**: a change under `docker/` needs the next `-r<N>` on the vmangos tag in
  `compose.yaml`, and a change under `datahost/` a new version in `datahost/Cargo.toml`, named by
  both compose files (`ci/pins.sh` checks they agree): `server-images.yml` refuses to publish other
  files under a tag the registry already has (unless run by hand with `republish`). A new
  `VMANGOS_COMMIT` from iNilla-server makes a new tag by itself.
- **The datahost's contract** with the app (`/server.json`, `POST /accounts`): the app reads it in
  `crates/inilla-core/src/connect.rs`.

What Vanillish changed in `server/`:

| file | change |
|---|---|
| `README.md`, `docs/OPERATIONS.md`, `docs/DESIGN.md`, `addons/README.md` | rewritten for Vanillish: the Mac first, the one-command setup, the prebuilt images, quick connect, sign-up; credits to iNilla-server |
| `setup-mac.sh`, `setup-linux.sh` (new) | the one-command setup: the server's files into `~/Vanillish-server`, Docker (Docker Desktop, OrbStack or Colima on a Mac; Docker Engine on Linux; Docker Desktop under WSL), the address, `.env`, the archives copied once into `vanillish-wowdata`, the images pulled and the first start followed, the address to type; `setup-linux.sh` hands over to `setup-mac.sh` |
| `compose.yaml` | project `vanillish`; the images `${VANILLISH_REGISTRY:-ghcr.io/klanthier}/vanillish-vmangos:<VMANGOS_COMMIT>-r1` and `vanillish-datahost:0.3.0`, pulled, with `build:` beside them for a tag the registry lacks; the archives from the external volume `vanillish-wowdata` unless `WOW_DATA_DIR` names a folder; `VMANGOS_DB_SHA256`, `BUILD_JOBS`; db-init's `SIGNUP_DB_PASSWORD`; the data host's `server.json` settings (`INILLA_SERVER_NAME`, `INILLA_CORE`, `INILLA_AUTH_PORT`, `INILLA_REALM_*`, `INILLA_WORLD_*`, `INILLA_MESSAGE`) and sign-up (`INILLA_SIGNUP`, `lan` by default; `INILLA_SIGNUP_DB_*`), started after db-init alone; `REALM_NAME` `Vanillish` |
| `compose.datahost.yaml` | project `vanillish-datahost`, the published image, the volume as the default data, sign-up off, the optional `server.json` settings |
| `.env.example` | `SIGNUP_DB_PASSWORD`, `INILLA_SIGNUP`, `INILLA_MESSAGE`, the volume in place of a required `WOW_DATA_DIR`, `VANILLISH_REGISTRY`, `VMANGOS_DB_SHA256`, `BUILD_JOBS`; the Vanillish names |
| `.dockerignore`, `datahost/.dockerignore` | no archive of any case (a `.part` too), no `Data/` or `WTF/` folder, no `.env` in a build context: the images are published |
| `docker/vmangos/Dockerfile` | native on amd64 and arm64: `-fsigned-char -ffp-contract=off` on every translation unit (checked from the compile database), `BUILD_JOBS`; the world dump checked against GitHub's digest or `VMANGOS_DB_SHA256`, on the build platform; vmangos' `LICENSE` in the image; Vanillish's labels |
| `docker/vmangos/bin/inilla-db-init` | the sign-up user `vanillish_signup` and its column grants, checked and reset when they differ; `REALM_NAME` `Vanillish` |
| `docker/vmangos/bin/inilla-lib.sh`, `inilla-{extract,mangosd,realmd}`, `vmangos-console` | `check_secret`, for the sign-up password too; shellcheck-clean |
| `datahost/` (`src/info.rs`, `src/accounts.rs` new; `src/main.rs`, `src/http.rs`, `Cargo.toml` 0.3.0 and its own `[workspace]`, `Cargo.lock`, `Dockerfile`) | `GET /server.json`; `POST /accounts` (the SRP6 salt and verifier into realmd's database as `vanillish_signup`, `INILLA_SIGNUP`, the token, the rate limits); a start with no game data; the request head bounded |
| `ci/` (new) | `check.sh` (the stack with no game data: the sign-up user's grants, `/server.json`, sign-up, an SRP6 logon), `accounts_e2e.py`, `srp6.py`, `setup-test.sh`, `pins.sh`, `image-state.sh`, `test_ci_tools.sh` |
| `.github/workflows/server-check.yml`, `server-images.yml` (new, outside `server/`) | the check on amd64 and arm64 runners; the multi-arch images published to GHCR |

---

*Below: iNilla's own UPSTREAM.md, unchanged.*

# iNilla and upstream benilla

iNilla is a fork of [samwhosung/benilla](https://github.com/samwhosung/benilla) that **merges**
upstream as it publishes (the way [wenilla](https://github.com/Arnesen/wenilla) does). iNilla's own
code lives in its own crates and in `crates/benilla-app/src/inilla/`; changes inside upstream
files ("carries") are kept few and small so a merge conflicts on as little as possible.

## Syncing

```sh
git remote add upstream https://github.com/samwhosung/benilla   # once
git fetch upstream
git checkout -b sync-upstream-$(date +%Y%m%d)
git merge upstream/main          # a real merge commit — never a rebase or a squashed copy
# resolve the carries below if they conflict, then:
cargo test -p inilla-core && cargo check -p benilla-app
source ios/toolchain/linux-env.sh && cargo check --target aarch64-apple-ios -p inilla-ios
```

Open a PR from the sync branch and merge it **with a merge commit** (keeping upstream as the second
parent is what keeps the next sync small).

## The carries

Every upstream file iNilla changes, and why. Each change is marked `iNilla` in a comment.

| file | change |
|---|---|
| `Cargo.toml` | `[profile.ios]` (release, no LTO, symbols kept) |
| `crates/benilla-app/Cargo.toml` | depends on `inilla-core`, `inilla-platform`, `ureq`, `serde_json`; `bevy_egui` no longer optional; `arboard` not on iOS; the Wayland clipboard's `cfg` excludes iOS; `raw-window-handle` and `winit` on iOS |
| `crates/benilla-app/src/lib.rs` | `mod inilla`, `pub use inilla_boot`, `add_plugins(inilla::InillaPlugin)` |
| `crates/benilla-app/src/bindings.rs`, `bindings/script_input.rs` | the synthetic seam `synth_hold`/`synth_release`/`synth_fire` and `Held::Synth` |
| `crates/benilla-app/src/textinput/clipboard.rs` | the iOS pasteboard backend |
| `crates/benilla-app/src/login/mod.rs` | `Attempt`/`send` `pub(crate)`, `in_flight()` — the launcher submits through the login screen's path |
| `crates/benilla-app/src/net/io.rs` | the world address passes through `inilla::world_address` (the realm override) |
| `crates/benilla-app/src/player/camera.rs` | `LookConfig::yaw_rate`/`pitch_rate` `pub(crate)` (controller look in radians) |
| `crates/benilla-app/src/debug_panel/mod.rs` | the egui lane's demand gate also reads `inilla::EguiWanted` |
| `crates/benilla-ui/src/script/mod.rs`, `script/plain.rs` | `eval_plain` (wenilla's) — Lua results as plain values for the console and the HUD icons |
| `third_party/lua-src/lua-5.1.5/loslib.c`, `BENILLA.md` | `os.execute` answers "not supported" on iOS, where `system()` does not exist |
| `.github/workflows/close-prs.yml` | removed (upstream closes every PR; this repository takes them) |
| `crates/benilla-dbc/src/localize.rs` (new), `lib.rs` | `localize` / `data_locale`: a non-English client's DBC text moved into the English column every reader names |
| `crates/benilla-formats/src/chain.rs`, `lib.rs` | `Chain::open` notes the install's language off `ChrRaces.dbc`; `Chain::read` localizes `.dbc` files of a non-English install; `data_locale()` |
| `crates/benilla-protocol/src/auth.rs` | `set_client_locale`: the logon challenge reports the game data's language instead of a fixed `enUS` |
| `crates/benilla-ui/src/script/client.rs`, `mod.rs`, `glue_core.rs` | `set_locale`: `GetLocale()` answers the game data's language in game and glue |
| `crates/benilla-app/src/addon_harness/render_tests.rs`, `use_probe_tests.rs` | the two tests whose fixtures are symlinks are `cfg(unix)`, so the app's tests build on Windows |
| `crates/benilla-ui/src/script/client.rs`, `script/model.rs`, `strings.rs`, `script/tooltip_item/render.rs` | `SetEuropeanNumbers`, which the German `Localization.lua` calls at `VARIABLES_LOADED`: `strings::decimal` / `fill_with` write the tooltips' reals (speed, DPS, ammo, cast time, cooldown) with a comma |

Upstream has no CI and never builds iOS, so an upstream change can break only the iOS build; the
`inilla-check` workflow's iOS type-check is what catches it.
