#!/usr/bin/env bash
# Vanillish: build libvanillish.so (the whole client) for Android with cargo-ndk, into the APK's
# jniLibs, and check it is a library GameActivity can load. Runs on Linux or macOS; needs rustup,
# cargo-ndk, python3 and the Android NDK that android/gradle.properties pins.
#
#   android/build-rust.sh                        # → android/app/src/main/jniLibs/arm64-v8a/libvanillish.so
#   VANILLISH_FEATURES=dev android/build-rust.sh # with benilla's dev instruments compiled in
#   VANILLISH_PROFILE=release android/build-rust.sh
#
# Then: (cd android && ./gradlew assembleRelease)   — or android/build-apk.sh for both steps.
#
# minSdk (cargo-ndk's -P) and the NDK come from android/gradle.properties, the lines
# app/build.gradle.kts reads, so the library is linked for exactly the API level the APK claims.
# After the build the library must:
#   - export android_main, GameActivity's JNI entry point, and every Java_* function the Java
#     sources declare `native` (android/tools/jni-natives.py) — a missing one is no load error but
#     an UnsatisfiedLinkError on its first call, mid-game;
#   - need no libc++_shared.so (GameActivity's glue links libc++ statically) and nothing else that
#     is not a platform library at minSdk: the APK ships libvanillish.so alone;
#   - have every LOAD segment 16 KB-aligned: devices with 16 KB pages refuse anything less.
# The copy in the APK loses its DWARF but keeps its symbol table (a backtrace in logcat still names
# functions); the untouched library is kept for symbolication in target/vanillish-out/symbols/.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() {
    echo "vanillish: $*" >&2
    exit 1
}
# One pin from android/gradle.properties.
pin() {
    local v
    v="$(sed -n "s/^vanillish\.$1=//p" "$ROOT/android/gradle.properties" | tr -d '\r' | tail -n 1)"
    [ -n "$v" ] || fail "android/gradle.properties has no vanillish.$1"
    printf '%s' "$v"
}

PROFILE="${VANILLISH_PROFILE:-android}"
FEATURES="${VANILLISH_FEATURES:-}"
# The Galaxy Z Fold line and every current Android phone: arm64. (x86_64 builds for the emulator,
# but app/build.gradle.kts's abiFilters packs arm64-v8a only.)
ABIS="${VANILLISH_ABIS:-arm64-v8a}"
MIN_SDK="$(pin minSdk)"
NDK_VERSION="$(pin ndkVersion)"
JNI="$ROOT/android/app/src/main/jniLibs"
OUT="$ROOT/target/vanillish-out"

# The pinned NDK: ANDROID_NDK_HOME if it is that release, else the SDK's side-by-side copy. (A CI
# runner's ANDROID_NDK_HOME is its image's default NDK, usually another release.)
ndk_revision() {
    sed -n 's/^Pkg\.Revision *= *//p' "$1/source.properties" 2>/dev/null | tr -d '\r'
}
NDK=""
if [ -n "${ANDROID_NDK_HOME:-}" ] && [ "$(ndk_revision "$ANDROID_NDK_HOME")" = "$NDK_VERSION" ]; then
    NDK="$ANDROID_NDK_HOME"
else
    for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Android/Sdk" "$HOME/android-sdk" "$HOME/Library/Android/sdk"; do
        if [ -n "$sdk" ] && [ "$(ndk_revision "$sdk/ndk/$NDK_VERSION")" = "$NDK_VERSION" ]; then
            NDK="$sdk/ndk/$NDK_VERSION"
            break
        fi
    done
    if [ -n "$NDK" ] && [ -n "${ANDROID_NDK_HOME:-}" ]; then
        echo "vanillish: ANDROID_NDK_HOME is NDK $(ndk_revision "$ANDROID_NDK_HOME"); using the pinned $NDK_VERSION ($NDK)"
    fi
fi
[ -n "$NDK" ] || fail "no Android NDK $NDK_VERSION (android/gradle.properties) — sdkmanager --install \"ndk;$NDK_VERSION\", or point ANDROID_NDK_HOME at it"
# cargo-ndk takes the first of these that is set and warns when the others disagree.
export ANDROID_NDK_HOME="$NDK" ANDROID_NDK_ROOT="$NDK"
unset ANDROID_NDK_PATH NDK_HOME
bins=("$NDK"/toolchains/llvm/prebuilt/*/bin)
BIN="${bins[0]}"
[ -x "$BIN/llvm-readelf" ] || fail "no llvm-readelf in $BIN"

# GameActivity's Java half (Gradle, vanillish.gamesActivity) must be the release whose C half
# android-activity compiles in; the pins pair the two, Cargo.lock decides the crate.
want="$(pin androidActivity)"
locked="$(awk '/^\[\[package\]\]/ { n = 0 }
               $0 == "name = \"android-activity\"" { n = 1; next }
               n && /^version = / { gsub(/version = |"/, ""); print; n = 0 }' Cargo.lock | sort -u | tr '\n' ' ')"
locked="${locked% }"
if [ "$locked" != "$want" ]; then
    fail "Cargo.lock has android-activity ${locked:-(none)}, but android/gradle.properties pairs" \
        "games-activity $(pin gamesActivity) with android-activity $want. Read android-activity's" \
        "README for the games-activity release it bundles, then update vanillish.androidActivity" \
        "and vanillish.gamesActivity together."
fi

command -v cargo-ndk >/dev/null || fail "no cargo-ndk — cargo install cargo-ndk --locked --version $(pin cargoNdk)"
command -v python3 >/dev/null || fail "no python3 (android/tools/jni-natives.py reads the Java natives)"

targets=()
for abi in $ABIS; do
    case "$abi" in
        arm64-v8a) triple=aarch64-linux-android ;;
        x86_64) triple=x86_64-linux-android ;;
        *) fail "unsupported ABI $abi (arm64-v8a, x86_64)" ;;
    esac
    rustup target add "$triple" >/dev/null 2>&1 || true
    targets+=(-t "$abi")
done

feature_args=()
if [ -n "$FEATURES" ]; then
    feature_args=(--features "$FEATURES")
fi

# The natives the Java side will call, before the long build: a parse error shows at once.
natives="$(python3 "$ROOT/android/tools/jni-natives.py" "$ROOT/android/app/src")" ||
    fail "android/tools/jni-natives.py could not list the Java natives"

echo "vanillish: building libvanillish.so for $ABIS (API $MIN_SDK, $PROFILE${FEATURES:+, features: $FEATURES}) with NDK $NDK_VERSION, $(cargo ndk --version)"
# A fresh jniLibs: nothing stale (another ABI, a library from another checkout) reaches the APK.
rm -rf "$JNI"
mkdir -p "$JNI" "$OUT"
# `${a[@]+"${a[@]}"}`: an empty array under `set -u` is an error in macOS's bash 3.2.
cargo ndk "${targets[@]}" -P "$MIN_SDK" -o "$JNI" \
    build --profile "$PROFILE" -p vanillish-android ${feature_args[@]+"${feature_args[@]}"}

rm -rf "$OUT/symbols"
mkdir -p "$OUT/symbols"
problems=0
problem() {
    echo "vanillish: $*" >&2
    problems=$((problems + 1))
}
for abi in $ABIS; do
    case "$abi" in
        arm64-v8a) triple=aarch64-linux-android ;;
        x86_64) triple=x86_64-linux-android ;;
    esac
    so="$JNI/$abi/libvanillish.so"
    [ -f "$so" ] || fail "cargo-ndk left no $so"

    # The symbols copy first, then the DWARF out of the APK's copy. The profile builds our crates
    # without debug info, so the DWARF is the precompiled std's line tables (~25 MB); the symbol
    # table that names functions in a backtrace stays.
    mkdir -p "$OUT/symbols/$abi"
    cp "$so" "$OUT/symbols/$abi/libvanillish.so"
    "$BIN/llvm-strip" --strip-debug "$so"

    # 1. Exported: what GameActivity's Java and our own Java resolve by name.
    exported="$("$BIN/llvm-nm" -D --defined-only --format=just-symbols "$so")"
    for sym in android_main Java_com_google_androidgamesdk_GameActivity_initializeNativeCode; do
        printf '%s\n' "$exported" | grep -Fxq "$sym" ||
            problem "$abi: $sym is not exported — GameActivity could not start the game (android_main is crates/vanillish-android's; the JNI entry comes with android-activity's game-activity feature)"
    done
    for sym in $natives; do
        printf '%s\n' "$exported" | grep -Fxq "$sym" ||
            problem "$abi: $sym is not exported — a Java \`native\` without its #[no_mangle] pub extern \"system\" fn in crates/vanillish-android/src/jni_exports.rs, or with a name that differs"
    done
    # An export no Java declares is harmless, but usually one half of a rename.
    for sym in $(printf '%s\n' "$exported" | grep '^Java_' | grep -v '^Java_com_google_androidgamesdk_' || true); do
        printf '%s\n' "$natives" | grep -Fxq "$sym" ||
            echo "vanillish: warning: $abi: $sym is exported but no Java class declares it native"
    done

    # 2. Needed libraries: the platform's own, at minSdk. No C++ runtime to ship.
    stubs="$(dirname "$BIN")/sysroot/usr/lib/$triple/$MIN_SDK"
    [ -d "$stubs" ] || fail "no API $MIN_SDK stubs in $stubs"
    needed="$("$BIN/llvm-readelf" -d "$so" | sed -n 's/.*(NEEDED).*\[\(.*\)\].*/\1/p')"
    for lib in $needed; do
        case "$lib" in
            libc++_shared.so | libc++.so)
                problem "$abi: needs $lib — GameActivity's glue links libc++ statically, so something (a crate's build script, cargo-ndk --link-libcxx-shared) asked for the shared C++ runtime, which the APK does not ship"
                ;;
            *)
                [ -f "$stubs/$lib" ] ||
                    problem "$abi: needs $lib, which is not a platform library at API $MIN_SDK (the APK ships libvanillish.so alone)"
                ;;
        esac
    done

    # 3. 16 KB pages: every LOAD segment's alignment a multiple of 0x4000 (NDK r28's default).
    aligns="$("$BIN/llvm-readelf" -lW "$so" | awk '$1 == "LOAD" { print $NF }')"
    [ -n "$aligns" ] || problem "$abi: no LOAD segments in $so"
    for align in $aligns; do
        case "$align" in
            0x*) [ $((align % 16384)) -eq 0 ] || problem "$abi: a LOAD segment is aligned to $align, not 16 KB" ;;
            *) problem "$abi: unreadable LOAD alignment '$align'" ;;
        esac
    done

    echo "vanillish: $abi: checked the exports (android_main, GameActivity, $(printf '%s\n' "$natives" | grep -c . || true) Java natives);" \
        "needs $(printf '%s\n' "$needed" | paste -sd, - | sed 's/,/, /g');" \
        "LOAD aligned $(printf '%s\n' "$aligns" | sort -u | paste -sd' ' -)"
    ls -lh "$so" "$OUT/symbols/$abi/libvanillish.so"
done
[ "$problems" -eq 0 ] || fail "$problems problem(s) with libvanillish.so (above)"

git -C "$ROOT" rev-parse --short HEAD >"$OUT/symbols/commit.txt" 2>/dev/null || echo unknown >"$OUT/symbols/commit.txt"
