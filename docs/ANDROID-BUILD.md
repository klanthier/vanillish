# Building and installing Vanillish.apk

## The short version

1. **Actions → vanillish-apk → Run workflow.** One Linux runner builds everything: about 35–45
   minutes the first time, about 15 once the cache is warm (estimates; the build is mostly Rust).
2. On the phone, open the run in the GitHub app or a browser and download its artifact
   `Vanillish-<commit>.apk` — the APK itself, not a zip — and open it.
3. Android asks once to let the browser (or My Files) install unknown apps: allow it. If Play
   Protect warns about an unknown app: *More details → Install anyway*.
4. First launch: Vanillish asks for the game data — see [ANDROID-DATA.md](ANDROID-DATA.md).

**Install updates over the installed app; never uninstall first.** Uninstalling deletes
Vanillish's folder — the game data, the settings, the SavedVariables — and Android installs an
update only when it is signed with the same key as the installed app ([Signing](#signing)).

**Releases** publish a GitHub pre-release with `Vanillish-<version>.apk` attached. The release text
comes first (its first line the title), then the build facts (commit, version, package, size,
SHA-256, the signing certificate's SHA-256), the install steps and GitHub's generated changelog.
A release must be signed with the release key: without the signing secrets it fails in seconds
rather than publish an APK nobody could update. Two ways to start one:

- **Actions → vanillish-apk → Run workflow** with `release` (e.g. `vanillish-v0.3.0`) and `notes`
  (the release text) filled in, variant `release`: GitHub creates the tag on the commit that was
  built.
- From a checkout, an annotated tag, whose message is the release text:

  ```sh
  git tag -a vanillish-v0.3.0 -m "Short title" -m "What changed, for players."
  git push origin vanillish-v0.3.0
  ```

The tag must look like `vanillish-vX.Y.Z` (a suffix such as `-rc1` is allowed); the APK's version
is the part after `vanillish-v`. iNilla's own `v*` tags build only the `.ipa` (`inilla-ipa.yml`
leaves `vanillish-v*` alone), so the two never race for the same release.

Release files do not count against the Actions storage quota; run artifacts do (the APK and the
symbols are kept 14 days).

## How the build goes

| step | what | why |
|---|---|---|
| `android/build-rust.sh` | the whole client → `libvanillish.so`, with cargo-ndk, into `android/app/src/main/jniLibs/arm64-v8a/` | the game is the Rust library; GameActivity loads it. |
| — its checks | the library exports `android_main`, GameActivity's entry and every Java `native` method's function; needs no `libc++_shared.so` and nothing that is not a platform library at minSdk; every LOAD segment 16 KB-aligned | a missing export is no load error but a crash at its first call, mid-game; the APK ships `libvanillish.so` alone; phones with 16 KB pages refuse anything less. |
| Gradle (`android/`) | the Java side, the manifest, the resources, R8, signing → the APK | the Activity and its helpers around the library. |
| `android/build-apk.sh` | both, then checks the APK (the library inside, zip alignment, `apksigner verify`), names it, writes the notes | one command, the same locally and on CI. |

The library is built with Cargo's `android` profile (release optimisation, no LTO, symbols kept).
The APK's copy loses only its DWARF: a panic's backtrace in logcat still names functions. The
untouched copy goes to `target/vanillish-out/symbols/` for crash symbolication.

## Building locally

On Linux or macOS.

### Toolchain

Every Android version is pinned once, in `android/gradle.properties`; `app/build.gradle.kts`,
`build-rust.sh` and the workflows read the same lines.

| what | version | pinned in |
|---|---|---|
| Rust | 1.98.1, with clippy and rustfmt; target `aarch64-linux-android` | `rust-toolchain.toml` (`build-rust.sh` adds the target) |
| cargo-ndk | 4.1.2 (any 4.x builds the same) | `vanillish.cargoNdk` |
| Android NDK | 28.2.13676358 (r28c) | `vanillish.ndkVersion` |
| SDK platform, build-tools | android-36, 36.0.0 | `vanillish.compileSdk`, `vanillish.buildTools` |
| minSdk, targetSdk | 31 (Android 12), 36 | `vanillish.minSdk`, `vanillish.targetSdk` |
| GameActivity | games-activity 4.4.0, paired with android-activity 0.6.1 | `vanillish.gamesActivity`, `vanillish.androidActivity` |
| Gradle, Android Gradle Plugin | 9.8.1 (the wrapper), 9.4.1 | `android/gradle/wrapper/gradle-wrapper.properties`, `android/build.gradle.kts` |
| JDK | 17 or newer (CI uses Temurin 21) | — |
| python3 | any 3 | `android/tools/jni-natives.py` reads the Java natives |

```sh
sdkmanager --install "ndk;28.2.13676358" "platforms;android-36" "build-tools;36.0.0"
cargo install cargo-ndk --locked --version 4.1.2
export ANDROID_HOME=$HOME/Android/Sdk    # or sdk.dir=… in android/local.properties
```

The scripts find the SDK through `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `android/local.properties`, or
`~/Android/Sdk`, `~/android-sdk`, `~/Library/Android/sdk`; the NDK as `ANDROID_NDK_HOME` if that is
the pinned release, otherwise in the SDK's `ndk/28.2.13676358`.

GameActivity comes in two halves that must be the same release: the Java one is the Maven
artifact Gradle fetches, the C one is compiled into the library by android-activity's build
script. `build-rust.sh` stops when `Cargo.lock`'s android-activity is not the pinned one; on an
upgrade (a Bevy update), read android-activity's README for the games-activity release it bundles
and change `vanillish.androidActivity` and `vanillish.gamesActivity` together.

### Building

```sh
android/build-apk.sh                          # → target/vanillish-out/Vanillish-<commit>.apk
android/build-apk.sh debug                    # → Vanillish-<commit>-debug.apk (package ….vanillish.debug)
VANILLISH_FEATURES=dev android/build-apk.sh   # with benilla's dev instruments (debug panel, perf HUD)
```

or the two halves by hand:

```sh
android/build-rust.sh                         # → android/app/src/main/jniLibs/arm64-v8a/libvanillish.so
(cd android && ./gradlew assembleRelease)     # → android/app/build/outputs/apk/release/
```

Gradle refuses to package an APK without the library ("run android/build-rust.sh"); compiling the
Java and lint need none.

`target/vanillish-out/` then holds the APK, `build-notes.md` (the build's facts and the install
steps, which a release carries), `build.env` (for CI), and `symbols/arm64-v8a/libvanillish.so` with
`symbols/commit.txt`.

A `debug` build is the same release-optimised library; only the Java side is debuggable, which
lets [intent extras](#environment-knobs-debug-builds) set the client's environment. Its package
id ends in `.debug`, so it installs beside a release build, as a separate app with its own folder.

| knob | does |
|---|---|
| `VANILLISH_FEATURES=dev` | benilla's dev instruments compiled in |
| `VANILLISH_PROFILE` | the Cargo profile (default `android`) |
| `VANILLISH_SKIP_RUST=1` | `build-apk.sh` packages the library already in `jniLibs` |
| `VANILLISH_VERSION_NAME`, `VANILLISH_VERSION_CODE` | override the version ([below](#versions)) |
| `VANILLISH_REQUIRE_RELEASE_KEY=1` | fail rather than produce a debug-signed APK |
| `VANILLISH_KEYSTORE`, `VANILLISH_KEYSTORE_PASSWORD`, `VANILLISH_KEY_ALIAS`, `VANILLISH_KEY_PASSWORD` | the release key ([Signing](#signing)) |

### Checking without an APK

What `vanillish-check` runs, rustfmt and lint aside — type-checks, no code generation:

```sh
python3 android/tools/test_jni_natives.py     # every Java `native` has its Rust export
cargo test -p inilla-core -p inilla-platform
cargo check -p benilla-app --no-default-features
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/28.2.13676358
cargo ndk -t arm64-v8a -P 31 check -p vanillish-android
cargo ndk -t arm64-v8a -P 31 clippy -p inilla-platform -p vanillish-android --no-deps -- -D warnings
(cd android && ./gradlew --no-daemon -q :app:compileReleaseJavaWithJavac)
```

A Java `native` method's Rust function lives in `crates/vanillish-android/src/jni_exports.rs`;
rename both in the same change, or the first test above fails.

## CI

| workflow | when | what |
|---|---|---|
| `vanillish-check` | every push (not for `*.md`, `docs/`, `ios/` alone) and PR | the JNI natives test; rustfmt over iNilla's and Vanillish's code; the engine-free tests; clippy (host, and the Android entry and platform layer for `aarch64-linux-android`); the desktop type-check; the Android type-check with cargo-ndk; the Java compiles; Android lint, which reports and never blocks (its report is an artifact for 7 days). |
| `vanillish-apk` | Run workflow, and `vanillish-v*` tags | the APK, on one `ubuntu-24.04` runner. |
| `inilla-check` | every push and PR | iNilla's gate, unchanged — its iOS type-check keeps the fork mergeable back into iNilla. |

**vanillish-apk's inputs**: `dev` (benilla's dev instruments), `variant` (`release`, or `debug` for
debuggable Java and the `.debug` package), `release` (a tag to publish as; empty for a test build),
`notes` (the release text). **Its artifacts**: the APK under its own name, not zipped (a phone opens
it straight from the download), and `Vanillish-symbols-<commit>` (the unstripped library), both
kept 14 days. The Rust build runs with no secret in its environment; the keystore is decoded to the
runner's temporary folder only for the packaging step, and only the separate release job may write
to the repository.

Linux minutes count 1× against a private repository's plan, and the Rust cache (keyed
`vanillish-apk-android-player` or `…-dev`) shares the repository's 10 GB with the `inilla-*`
workflows'.

## Signing

**One key, forever.** Android installs an update only over an app signed with the same key. An APK
signed with another key cannot update the installed Vanillish, and the way past that —
uninstalling — deletes its folder: the game data, the settings, the SavedVariables. So releases are
signed with one Vanillish release key, kept safe, and never changed.

Make it once, and keep an offline backup of the file and its password:

```sh
keytool -genkeypair -keystore vanillish.jks -storetype PKCS12 -alias vanillish \
    -keyalg RSA -keysize 4096 -validity 36500 -dname "CN=Vanillish"
```

A PKCS12 keystore (keytool's default) has one password for the store and the key.

**On GitHub**: four repository secrets (Settings → Secrets and variables → Actions):

| secret | value |
|---|---|
| `VANILLISH_KEYSTORE_BASE64` | the keystore file, base64: `base64 -w0 vanillish.jks` (macOS: `base64 -i vanillish.jks`) |
| `VANILLISH_KEYSTORE_PASSWORD` | its password |
| `VANILLISH_KEY_ALIAS` | the key's alias (`vanillish` above) |
| `VANILLISH_KEY_PASSWORD` | optional: the key's own password, when it has one (a PKCS12 key has none) |

or from a checkout: `base64 -w0 vanillish.jks | gh secret set VANILLISH_KEYSTORE_BASE64`, and
`gh secret set VANILLISH_KEYSTORE_PASSWORD` (it asks). Without them a test build is signed with the
runner's throwaway debug key — a new one every run — and the run says so in a warning and in its
summary: such an APK can go only on a phone without Vanillish.

**Locally**: the same four as environment variables, with `VANILLISH_KEYSTORE` the keystore's path,
or a file `android/keystore.properties` (ignored by version control; keep the keystore itself
outside the repository — only `android/` ignores `*.jks`):

```properties
storeFile=/absolute/path/to/vanillish.jks
storePassword=…
keyAlias=vanillish
keyPassword=
```

(An empty or missing `keyPassword` uses the store's.) A relative `storeFile` is taken from
`android/app/`. With neither, builds are signed with this machine's debug key and say so loudly;
`VANILLISH_REQUIRE_RELEASE_KEY=1` makes that an error. With a key set, the debug variant is signed
with it too, so one debug build updates the last.

The certificate's SHA-256 is in each release's notes and in `build-notes.md`; check an APK with
`$ANDROID_HOME/build-tools/36.0.0/apksigner verify --print-certs Vanillish-….apk`.

## Versions

| | release | any other build |
|---|---|---|
| `versionName` | the tag's version (`vanillish-v0.3.0` → `0.3.0`) | the workspace version and the commit: `0.2.0+c36035b7` |
| `versionCode` | the commit's time, in minutes since 1970 | the same |

So a build of a newer commit installs over a build of an older one whether it was made locally or
on CI, and a local build and a CI build of one commit agree. A build of an older commit is a
downgrade, which Android refuses (`adb install -r -d` allows it for a debug build only).
`VANILLISH_VERSION_NAME` and `VANILLISH_VERSION_CODE` override both.

## Installing

- **On the phone**: download the APK — from a release, or a run's artifacts — and open it. Allow the
  browser or My Files to install unknown apps when Android asks; past Play Protect's warning with
  *More details → Install anyway*.
- **From a computer**, with USB debugging on (Settings → About phone → Software information → tap
  Build number seven times; then Settings → Developer options → USB debugging):

  ```sh
  adb install -r target/vanillish-out/Vanillish-<commit>.apk
  ```

- **Updating**: install the new APK over the old one. The game data, the settings, the layouts and
  the saved passwords stay. An APK signed with another key fails to install ("App not installed",
  or `INSTALL_FAILED_UPDATE_INCOMPATIBLE` from adb); if you must change keys anyway, copy
  `benilla-config` off the phone over USB first, uninstall, install, copy it back, and bring the
  game data in again.
- **A debug build** installs beside the release one, as a separate app (`….vanillish.debug`) with
  its own folder and its own copy of the data.

## Debugging on Android

The tools of [DEBUGGING.md](DEBUGGING.md) all work — the session log, the remote console, the
desktop rehearsal — plus Android's own.

### logcat

```sh
adb logcat -s vanillish                                              # the client's own lines, and the Java side's
adb logcat --pid=$(adb shell pidof -s io.github.klanthier.vanillish)  # everything the process logs
```

Everything the client prints — the boot lines (`inilla: …`), a panic with its backtrace — and every
line of the Java side is logged under the tag `vanillish`. Bevy's log lines (`info!`, `warn!`)
reach logcat under tags of their own (the line's source location), so the second command is the
one that shows both (for a debug build, the package ends in `.debug`). The session log has
everything, in order.

### The session log

As on iOS, every launch writes `benilla-config/Diagnostics/session-<UTC time>.log` (Settings →
Debug, on by default), the last 10 are kept, and a panic leaves `crash-<unix>.txt` beside them. To
get them:

- **Settings → Debug → Share session log…** (or **Share…** in the log viewer) hands the current log
  to any app — mail, a chat, a cloud drive. The viewer's **Copy** takes only the last ~200 KB of
  lines (Android's clipboard is small); Share takes the whole file.
- From a PC over USB: Internal storage → Android → data → io.github.klanthier.vanillish → files →
  benilla-config → Diagnostics.
- With adb: `adb pull /sdcard/Android/data/io.github.klanthier.vanillish/files/benilla-config/Diagnostics`

### The remote console

Settings → Debug → **Remote console on the local network**, then the address it shows in a browser
on the same Wi-Fi, or on a computer joined to the phone's hotspot ([DEBUGGING.md](DEBUGGING.md#2-on-the-same-wi-fi-the-remote-console)).
Over USB, with no network at all:

```sh
adb forward tcp:7878 tcp:7878
# then http://localhost:7878/?token=<the token Settings → Debug shows>
```

### Environment knobs (debug builds)

A debug build takes the client's environment variables as intent extras — names starting with
`WOW_`, `BENILLA_`, `INILLA_` or `VANILLISH_`, and `RUST_LOG`, `RUST_BACKTRACE` — set before the
library loads. They must be strings (`--es`), and `-S` stops a running game first: the game runs
once per process, so the extras only reach a fresh one. A release build ignores them.

```sh
adb shell am start -S \
    -n io.github.klanthier.vanillish.debug/io.github.klanthier.vanillish.VanillishActivity \
    --es WOW_HOST 192.168.1.10 --es WOW_USER alice --es WOW_PASS secret \
    --es RUST_LOG info,benilla_app=debug
```

The useful ones: `WOW_HOST` (the server for this session), `WOW_USER` / `WOW_PASS` (log in without
typing), `RUST_LOG` (the log filter), `VANILLISH_PIPELINED=1` (Bevy's pipelined rendering back on,
to measure what turning it off costs). `BENILLA_HOME` and `WOW_DATA` are set by the app itself and
cannot be moved this way.

### Native crashes

A Rust panic writes its message and backtrace to logcat and the session log (`RUST_BACKTRACE=1` is
the default here). A native crash — a signal in Vulkan's driver, say — leaves a tombstone in logcat
instead; symbolicate it with the NDK's `ndk-stack` and the unstripped library of the same commit
(`symbols/commit.txt`; for a CI build, the run's `Vanillish-symbols-<commit>` artifact):

```sh
adb logcat -d | "$ANDROID_NDK_HOME/ndk-stack" -sym target/vanillish-out/symbols/arm64-v8a
```

The APK's own library keeps its symbol table, so a backtrace already names functions; the
unstripped copy adds the standard library's line tables.

### On the desktop

The Android parts rehearse on Linux, with the same code as [DEBUGGING.md](DEBUGGING.md#3-on-the-desktop-rehearse-the-phone)'s
touch HUD:

```sh
# the inner screen half folded on a table (flat: lying open; book: a vertical hinge):
INILLA_TOUCH=1 INILLA_FOLD=half WOW_WIN=954x859 WOW_DATA=/path/to/WoW/Data cargo play
# Import from folder…, with that folder as the "picked" one:
INILLA_CONTAINER=/tmp/fakephone INILLA_PICK="/path/to/World of Warcraft" cargo play
```

The hardware mouse, the soft keyboard, Back and the session service have no rehearsal: their calls
do nothing off Android.

## When something is off

- "App not installed" on an update → it is signed with another key than the installed app
  ([Signing](#signing)).
- The app opens and closes at once → `adb logcat -s vanillish` shows its last words; the session
  log from the attempt is in `benilla-config/Diagnostics`.
- `build-rust.sh` stops on the android-activity pin → `Cargo.lock` moved (a Bevy update); change the
  two pins together ([Toolchain](#toolchain)).
- `build-rust.sh` names a `Java_…` symbol that is not exported → a Java `native` method without its
  function in `jni_exports.rs`, or with another name.
- "Warden" in the login error → the server runs Warden; benilla cannot join it.
- Choppy or hot → Settings → Debug → the perf overlay (frame rate, memory, heat), and a lower preset
  or render resolution ([ANDROID.md](ANDROID.md#frame-rate-120-hz-and-the-presets)).
