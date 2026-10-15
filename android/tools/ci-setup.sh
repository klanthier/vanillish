#!/usr/bin/env bash
# Vanillish CI: what a GitHub Linux runner lacks for the Android build, at the versions
# android/gradle.properties pins — the NDK (the image carries other releases), the SDK platform and
# build-tools, the Rust target, cargo-ndk — and the pins themselves for the steps after it.
#
#   android/tools/ci-setup.sh        # in a workflow step, after the Rust toolchain and the caches
#
# Exports to $GITHUB_ENV (when set): ANDROID_NDK_HOME/ANDROID_NDK_ROOT (the pinned NDK, over the
# image's default) and VANILLISH_MIN_SDK (cargo-ndk's -P for `cargo ndk check`). Anything already
# in place is left alone, so a warm run spends seconds here.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
pin() {
    local v
    v="$(sed -n "s/^vanillish\.$1=//p" "$ROOT/android/gradle.properties" | tr -d '\r' | tail -n 1)"
    [ -n "$v" ] || { echo "vanillish: android/gradle.properties has no vanillish.$1" >&2; exit 1; }
    printf '%s' "$v"
}
MIN_SDK="$(pin minSdk)"
COMPILE_SDK="$(pin compileSdk)"
BUILD_TOOLS="$(pin buildTools)"
NDK_VERSION="$(pin ndkVersion)"
CARGO_NDK="$(pin cargoNdk)"

SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[ -n "$SDK" ] && [ -d "$SDK" ] || { echo "vanillish: no Android SDK (ANDROID_HOME)" >&2; exit 1; }
SDKMANAGER="$SDK/cmdline-tools/latest/bin/sdkmanager"

wanted=()
[ -f "$SDK/ndk/$NDK_VERSION/source.properties" ] || wanted+=("ndk;$NDK_VERSION")
[ -d "$SDK/platforms/android-$COMPILE_SDK" ] || wanted+=("platforms;android-$COMPILE_SDK")
[ -x "$SDK/build-tools/$BUILD_TOOLS/apksigner" ] || wanted+=("build-tools;$BUILD_TOOLS")
if [ "${#wanted[@]}" -gt 0 ]; then
    [ -x "$SDKMANAGER" ] || { echo "vanillish: no sdkmanager at $SDKMANAGER" >&2; exit 1; }
    echo "vanillish: sdkmanager --install ${wanted[*]}"
    yes 2>/dev/null | "$SDKMANAGER" --licenses >/dev/null || true
    "$SDKMANAGER" --install "${wanted[@]}" >/dev/null
fi
NDK="$SDK/ndk/$NDK_VERSION"
[ -f "$NDK/source.properties" ] || { echo "vanillish: NDK $NDK_VERSION did not install" >&2; exit 1; }

rustup target add aarch64-linux-android

# cargo-ndk: from the cache (rust-cache keeps ~/.cargo/bin) unless it is another version.
if ! cargo ndk --version 2>/dev/null | grep -qx "cargo-ndk $CARGO_NDK"; then
    cargo install cargo-ndk --locked --force --version "$CARGO_NDK"
fi
cargo ndk --version

if [ -n "${GITHUB_ENV:-}" ]; then
    {
        echo "ANDROID_NDK_HOME=$NDK"
        echo "ANDROID_NDK_ROOT=$NDK"
        echo "VANILLISH_MIN_SDK=$MIN_SDK"
    } >>"$GITHUB_ENV"
fi
echo "vanillish: NDK $NDK_VERSION, android-$COMPILE_SDK, build-tools $BUILD_TOOLS, minSdk $MIN_SDK"
