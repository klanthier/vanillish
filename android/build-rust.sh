#!/usr/bin/env bash
# Vanillish: build libvanillish.so (the whole client) for Android with cargo-ndk, into the APK's
# jniLibs. Runs on Linux or macOS; needs rustup, cargo-ndk and the Android NDK.
#
#   android/build-rust.sh                        # → android/app/src/main/jniLibs/arm64-v8a/libvanillish.so
#   VANILLISH_FEATURES=dev android/build-rust.sh # with benilla's dev instruments compiled in
#   VANILLISH_PROFILE=release android/build-rust.sh
#
# Then: (cd android && ./gradlew assembleRelease)   — or android/build-apk.sh for both steps.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="${VANILLISH_PROFILE:-android}"
FEATURES="${VANILLISH_FEATURES:-}"
# The Galaxy Z Fold line and every current Android phone: arm64. x86_64 builds for the emulator.
ABIS="${VANILLISH_ABIS:-arm64-v8a}"
# Must match the Gradle minSdk (AThermal and VibratorManager are linked directly).
API="${VANILLISH_API:-31}"
OUT="$ROOT/android/app/src/main/jniLibs"

if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Android/Sdk" "$HOME/android-sdk" "$HOME/Library/Android/sdk"; do
        if [ -n "$sdk" ] && [ -d "$sdk/ndk" ]; then
            ANDROID_NDK_HOME="$(ls -d "$sdk"/ndk/* | sort -V | tail -n 1)"
            break
        fi
    done
    export ANDROID_NDK_HOME
fi
if [ -z "${ANDROID_NDK_HOME:-}" ] || [ ! -d "$ANDROID_NDK_HOME" ]; then
    echo "vanillish: no Android NDK found — set ANDROID_NDK_HOME (sdkmanager 'ndk;28.2.13676358')" >&2
    exit 1
fi
command -v cargo-ndk >/dev/null || { echo "vanillish: cargo install cargo-ndk" >&2; exit 1; }

targets=()
for abi in $ABIS; do
    targets+=(-t "$abi")
    case "$abi" in
        arm64-v8a) rustup target add aarch64-linux-android >/dev/null 2>&1 || true ;;
        x86_64) rustup target add x86_64-linux-android >/dev/null 2>&1 || true ;;
    esac
done

feature_args=()
if [ -n "$FEATURES" ]; then
    feature_args=(--features "$FEATURES")
fi

echo "vanillish: building libvanillish.so for $ABIS (API $API, $PROFILE${FEATURES:+, features: $FEATURES}) with $(basename "$ANDROID_NDK_HOME")"
mkdir -p "$OUT"
cargo ndk "${targets[@]}" -P "$API" -o "$OUT" \
    build --profile "$PROFILE" -p vanillish-android "${feature_args[@]}"

for abi in $ABIS; do
    ls -lh "$OUT/$abi/libvanillish.so"
done
git -C "$ROOT" rev-parse --short HEAD >"$OUT/commit.txt" 2>/dev/null || echo unknown >"$OUT/commit.txt"
