# Vanillish and upstream iNilla / benilla

Vanillish is a fork of [Martze96/iNilla](https://github.com/Martze96/iNilla), which is itself a fork
of [samwhosung/benilla](https://github.com/samwhosung/benilla) that merges upstream as it
publishes (below). Vanillish merges both the same way: real merge commits, never a rebase or a
squashed copy. Its own code is in its own places — `crates/vanillish-android`, `android/`,
`crates/inilla-platform/src/android/`, the `vanillish-*` workflows — and in new files inside
iNilla's crates; changes inside iNilla's or benilla's files are carries (listed below), kept few
and small so a merge conflicts on as little as possible.

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

## The carries

Every file of benilla's and of iNilla's that Vanillish changes, and why. In benilla's files each
change is marked `Vanillish:` in a comment (`Cargo.lock` aside: it is generated); in iNilla's, each
change that adds Android behaviour is.

**In upstream benilla's files:**

| file | change |
|---|---|
| `Cargo.toml` | `[profile.android]` (release, no LTO, symbols kept), beside iNilla's `[profile.ios]` |
| `Cargo.lock` | the `vanillish-android` package; `jni` and `ndk-context` for `inilla-platform`. After a conflict, take the incoming one and let a `cargo check` add these back, rather than merge it by hand |
| `crates/benilla-app/Cargo.toml` | `arboard` not on Android either: it has no backend there (the clipboard is `inilla-platform`'s) |
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

**In iNilla's files:**

| file | change |
|---|---|
| `README.md`, `AGENTS.md`, `docs/UPSTREAM.md` | a Vanillish section on top; iNilla's text below it unchanged |
| `.github/workflows/inilla-ipa.yml` | tags `v*` but not `vanillish-v*`, which `vanillish-apk.yml` builds |
| `crates/inilla-core/src/lib.rs` | `mod fold`, `mod import` |
| `crates/inilla-core/src/settings.rs` | `tabletop_split`, `inner_any_orientation`, `stay_connected`, `last_game_mode`, `hide_touch_with_mouse`; `GameMode` and `follow_game_mode` (Game Dashboard picks the preset), tested |
| `crates/inilla-core/src/touch.rs` | the screen split at a hinge (the world above, the deck below) and the deck's look pad (`park`, public for the controller's look); pen hover, with no cursor over the deck; mouse buttons 3–5; the layout's screen class; the hub's radii; on the inner screen and the tabletop the radial at the thumb and capped at 170 pt (a phone class, so every iOS device, keeps iNilla's size) |
| `crates/inilla-core/src/data.rs` | the build-mismatch message says "this client", not "iNilla" |
| `crates/inilla-platform/Cargo.toml` | `jni` 0.21 and `ndk-context` on Android |
| `crates/inilla-platform/src/lib.rs` | `mod android`, `mod picker`, `mod pointer` |
| `crates/inilla-platform/src/device.rs` | an Android arm for every call; new calls (no-ops on iOS): `fold`, `fold_in_window` (`$INILLA_FOLD` on a desktop), `set_frame_rate`, `refresh_rate`, `game_mode`, `take_back_presses`, `set_session_service`, `set_orientation`, `set_gesture_exclusion`, `relaunch`, `quit`, `thermal_headroom` |
| `crates/inilla-platform/src/paths.rs` | the Android roots (`Android/data/<package>/files`, `.vanillish/Data`), `Host::Android`, `android_package`; `storage_unavailable`: shared storage did not answer at launch (asked again for about three seconds), so the roots are a private stand-in |
| `crates/inilla-platform/src/keychain.rs` | Android: sealed with an Android Keystore key, in private storage; `Unsealed` and `judge` (tested): only a key gone for good forgets the file, a busy Keystore keeps it |
| `crates/inilla-platform/src/pasteboard.rs` | Android's `ClipboardManager` |
| `crates/inilla-platform/src/audio.rs` | Android: audio focus for "mix with other apps" |
| `crates/inilla-platform/src/gamepad.rs` | `NATIVE` (the OS reports the pads: iOS and Android); `mod android` |
| `crates/inilla-platform/src/stdio.rs` | Android: the captured output goes to logcat (tracing's own lines excepted), each line's head cut on a char boundary; everywhere, fds 1 and 2 are pointed back at the original stderr (or `/dev/null`) when the reader thread ends, so a later print never meets a dead pipe |
| `crates/benilla-app/src/inilla/mod.rs` | the `android`, `screen`, `host_pointer` modules; `touch_device()` true on Android; `Inilla::other_layouts` |
| `crates/benilla-app/src/inilla/boot.rs` | Android's log filter, `TMPDIR`, `RUST_BACKTRACE`; off iOS, the launch-time import only as a rename of a settled, whole folder (`import::launch_guard`) |
| `crates/benilla-app/src/inilla/console.rs` | the addresses ranked for Android's interface names too: Wi-Fi, then the hotspot and USB tethering, then anything else in the system's order, a VPN last; cellular data and its 464XLAT address (`v4-*`) dropped; the page under the app's name |
| `crates/benilla-app/src/inilla/data.rs` | the folder import job, which refuses a picked folder whose files changed within `SETTLE`; Check for copied data's settled and cut-short guards off iOS; their messages; `needs_restart` also when a job added, removed or replaced an archive since the boot; no job but the manifest's while `paths::storage_unavailable` |
| `crates/benilla-app/src/inilla/hud.rs` | `HudScreen`; the tabletop's crease and deck; the chat bar's composing word; the controller radial above the hinge |
| `crates/benilla-app/src/inilla/icons.rs` | button tooltips anchored over the world's rect (above the hinge on a tabletop), offset by `UIParent`'s edges, in its own units |
| `crates/benilla-app/src/inilla/mouse.rs` | the middle, back and forward buttons |
| `crates/benilla-app/src/inilla/pad.rs` | `gamepad::NATIVE` in place of `cfg!(target_os = "ios")`; on a tabletop the cursor kept to the world above the hinge, and the look pressed where the touch look parks |
| `crates/benilla-app/src/inilla/platform.rs` | `ImePurpose`, `BackPresses`; Android's safe area, re-read after a screen change; the screen kept on during a data job; iNilla's own fields raise the keyboard; the interface scale per screen class; the preset's CVars only when the preset changes; `Pacing` (the frame cap stands down when the display paces); `clear_the_island` for a top cut-out and the hinge, and while it insets, the stock open-bag stack (`updateContainerFrameAnchors`) measuring `UIParent` instead of the screen (`island_chunk`, tested) |
| `crates/benilla-app/src/inilla/store.rs` | a layout file per screen class; `remove_layout` |
| `crates/benilla-app/src/inilla/touch.rs` | `HudScreen`; the HUD steps aside for a hardware mouse or keyboard; pen hover |
| `crates/benilla-app/src/inilla/ui/mod.rs` | the words from `words.rs`; `restart()` relaunches; Back closes iNilla's screens first; each field's `ImePurpose`; the layout editor handed the layout's own screen (`HudScreen.layout_screen`) |
| `crates/benilla-app/src/inilla/ui/settings.rs` | the Android rows, the frame rates, the Touch tab's screen class and tabletop switch, no silent-switch row, Share session log, Reset all resets every screen's layout |
| `crates/benilla-app/src/inilla/ui/setup.rs` | Import from folder…, Resume, the free space, Pause for the copy; the notice in place of the setup when shared storage did not answer at launch; the words |
| `crates/benilla-app/src/inilla/ui/launcher.rs` | the words; password and address fields |
| `crates/benilla-app/src/inilla/ui/logs.rs` | Android's copy capped to the last lines; Share… |
| `crates/benilla-app/src/inilla/ui/editor.rs` | laid out on the screen it is handed in place of the window's safe area (on a tabletop split at the hinge, the elements on the deck, at the login and character screens too); the deck outlined on a tabletop; a reset puts back this screen's own layout |

Vanillish's new files in iNilla's crates conflict with nothing unless iNilla adds one of the same
name: `inilla-core/src/{fold,import}.rs`, `inilla-platform/src/{picker,pointer}.rs`,
`inilla-platform/src/gamepad/android.rs`, `inilla-platform/src/android/`, and in
`benilla-app/src/inilla/`: `android.rs`, `screen.rs`, `host_pointer.rs`, `ui/words.rs`.

Neither benilla nor iNilla builds for Android, so nothing there notices a change that breaks the
Android build; `vanillish-check`'s Android type-check is what catches it.

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
