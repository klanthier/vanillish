# Vanillish (this fork)

This repository is **Vanillish**, iNilla built for Android (the Galaxy Z Fold8 Ultra first). Ours:
`crates/vanillish-android` (the app's `libvanillish.so` and its JNI exports), `android/` (the Gradle
project, the Java side, the build scripts), `crates/inilla-platform/src/android/`, the new Android
modules in iNilla's crates (`inilla-core`'s `fold`, `import`, `connect` and `admin`,
`inilla-platform`'s `gamepad/android`, `pointer`, `picker`, `benilla-app`'s
`inilla::{android,screen,host_pointer,connect,admin}` and `inilla/ui/{words,connect,admin}.rs`),
`server/` (iNilla-server, taken in at 990cc15) and the `vanillish-*` and `server-*` workflows.
Everything else is iNilla's or upstream benilla's, merged in (`docs/UPSTREAM.md`). Read
`docs/ANDROID.md` first, then `docs/ANDROID-BUILD.md`, and `server/README.md` for the server.

- Rust → Java: static methods on `io.github.klanthier.vanillish.Bridge`, through
  `inilla_platform::android::{call_bridge, call_bridge_void, with_env}`; never `FindClass`, never a
  cached Activity. Java → Rust: `static native` methods whose functions live in
  `crates/vanillish-android/src/jni_exports.rs` and only pass plain values on into
  `inilla-platform`. JNI code never panics. A Java class or member Rust reaches by name needs a
  `-keep` in `android/app/proguard-rules.pro`.
- `inilla-platform` takes no Bevy and no android-activity; what needs Bevy's `AndroidApp` lives in
  `crates/benilla-app/src/inilla/`. Engine-free logic goes in `inilla-core` with a unit test.
- A change inside an iNilla or benilla file is a carry: minimal, marked `Vanillish:`, listed in
  `docs/UPSTREAM.md`, iOS behaviour unchanged. A sentence that names the platform goes in
  `inilla/ui/words.rs`.
- Before pushing, besides iNilla's checks below (what `vanillish-check` runs; the NDK is
  28.2.13676358):

  ```sh
  cargo test -p inilla-core -p inilla-platform
  cargo clippy -p inilla-core -p inilla-platform --all-targets -- -D warnings
  cargo clippy -p benilla-app --all-targets -- -D warnings
  cargo check -p benilla-app --no-default-features
  cargo ndk -t arm64-v8a -P 31 check -p vanillish-android
  python3 android/tools/test_jni_natives.py
  (cd android && ./gradlew --no-daemon -q :app:compileReleaseJavaWithJavac)
  ```

  and when `server/` changed (`check.sh` needs Docker: the stack with no game data, sign-up and an
  SRP6 logon, as `server-check` runs it on amd64 and arm64):

  ```sh
  bash server/ci/setup-test.sh
  cargo test --manifest-path server/datahost/Cargo.toml
  server/ci/check.sh
  ```

---

*Below: iNilla's own AGENTS.md, unchanged. Its rules hold here too; "this repository" there is
iNilla, which Vanillish is built on.*

# iNilla (this fork)

This repository is **iNilla**, the iOS build of benilla: `crates/inilla-*`, `ios/`,
`crates/benilla-app/src/inilla/` and the `inilla-*` workflows are ours; everything else is
upstream benilla, merged in (see `docs/UPSTREAM.md` for the carries and how to sync). Read
`docs/INILLA.md` first, then `docs/IOS-BUILD.md` and `docs/DEBUGGING.md`.

- Engine-free logic goes in `inilla-core` with a unit test (`cargo test -p inilla-core`).
- OS calls go in `inilla-platform`, behind a function that compiles on every target.
- A change inside an upstream file is a carry: keep it minimal, mark it `iNilla`, list it in
  `docs/UPSTREAM.md`.
- Before pushing: `cargo check -p benilla-app` and
  `source ios/toolchain/linux-env.sh && cargo check --target aarch64-apple-ios -p inilla-ios`.
- Never commit anything from a WoW install (the `.MPQ` archives, extracted assets).

---

# benilla

A from-scratch World of Warcraft 1.12.1 client in Rust and Bevy. The reference client is the
spec: benilla is a faithful, modern implementation of it, not a place to get creative.

Read, in this order:

1. `docs/METHOD.md` — the rules and how we work. Binding for every session.
2. `docs/MAP.md` — what is built right now, generated from the code. Orient here before
   touching anything.
3. `docs/CONTRIBUTING.md` — what gets in and how a change is judged.

The rest of the root is code and data: `crates/` the workspace, `reference/` the 1.12 name
catalogues the tests check against, `scripts/` the gates and instruments, `third_party/`
vendored code.

@docs/METHOD.md
