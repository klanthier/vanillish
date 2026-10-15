#!/usr/bin/env bash
# Vanillish: the whole APK in one go — the Rust library, then Gradle.
#
#   android/build-apk.sh            # release APK (signed with android/keystore.properties or the
#                                   # VANILLISH_KEY* env, else the debug key) → target/vanillish-out/
#   android/build-apk.sh debug      # debug APK (id suffix .debug: installs beside a release build)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VARIANT="${1:-release}"

"$ROOT/android/build-rust.sh"

cd "$ROOT/android"
GRADLE=./gradlew
[ -x "$GRADLE" ] || GRADLE=gradle
task="assemble$(echo "${VARIANT:0:1}" | tr '[:lower:]' '[:upper:]')${VARIANT:1}"
"$GRADLE" --no-daemon "$task"

OUT="$ROOT/target/vanillish-out"
mkdir -p "$OUT"
short="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
apk="$(ls app/build/outputs/apk/"$VARIANT"/*.apk | head -n 1)"
cp "$apk" "$OUT/Vanillish-$short-$VARIANT.apk"
ls -lh "$OUT/Vanillish-$short-$VARIANT.apk"
