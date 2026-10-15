#!/usr/bin/env bash
# Vanillish: the whole APK in one go — the Rust library (build-rust.sh), then Gradle — then checked
# and described. Linux or macOS.
#
#   android/build-apk.sh            # release APK → target/vanillish-out/Vanillish-<commit>.apk
#   android/build-apk.sh debug      # debug APK (id suffix .debug: installs beside a release build)
#
# Signing: the release key from the environment (VANILLISH_KEYSTORE = the keystore's path,
# VANILLISH_KEYSTORE_PASSWORD, VANILLISH_KEY_ALIAS, optional VANILLISH_KEY_PASSWORD) or
# android/keystore.properties; without one, this machine's debug key — said loudly, because such an
# APK cannot update an install signed with any other key, and uninstalling deletes the app's data.
#
# Knobs:
#   VANILLISH_VERSION_NAME, VANILLISH_VERSION_CODE   override Gradle's (the workspace version + the
#                                   commit, and the commit's time in minutes: app/build.gradle.kts)
#   VANILLISH_SKIP_RUST=1           package the library already in jniLibs (build-rust.sh ran)
#   VANILLISH_REQUIRE_RELEASE_KEY=1 fail rather than ship a debug-signed APK (CI releases)
#   VANILLISH_CI_BUILD              a CI run number, for the notes
#   + build-rust.sh's: VANILLISH_FEATURES, VANILLISH_PROFILE
#
# Writes to target/vanillish-out/: the APK, build-notes.md (the build's facts and the install
# steps, which a release carries) and build.env (APK=, VERSION_NAME=, SIGNED_WITH=, … for CI).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VARIANT="${1:-release}"
OUT="$ROOT/target/vanillish-out"
PACKAGE=io.github.klanthier.vanillish

fail() {
    echo "vanillish: $*" >&2
    exit 1
}
pin() {
    local v
    v="$(sed -n "s/^vanillish\.$1=//p" "$ROOT/android/gradle.properties" | tr -d '\r' | tail -n 1)"
    [ -n "$v" ] || fail "android/gradle.properties has no vanillish.$1"
    printf '%s' "$v"
}
# The Android release a minSdk is, for the notes.
android_name() {
    case "$1" in
        31) echo "Android 12" ;; 32) echo "Android 12L" ;; 33) echo "Android 13" ;;
        34) echo "Android 14" ;; 35) echo "Android 15" ;; 36) echo "Android 16" ;;
        37) echo "Android 17" ;; *) echo "Android" ;;
    esac
}
sha256() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

case "$VARIANT" in
    release | debug) ;;
    *) fail "variant must be release or debug, not '$VARIANT'" ;;
esac

# The SDK, for Gradle (unless android/local.properties names it) and for apksigner/zipalign.
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [ -z "$SDK" ] && [ -f "$ROOT/android/local.properties" ]; then
    SDK="$(sed -n 's/^sdk\.dir=//p' "$ROOT/android/local.properties" | tr -d '\r')"
fi
if [ -z "$SDK" ]; then
    for sdk in "$HOME/Android/Sdk" "$HOME/android-sdk" "$HOME/Library/Android/sdk"; do
        if [ -d "$sdk/platforms" ]; then
            SDK="$sdk"
            break
        fi
    done
fi
[ -n "$SDK" ] && [ -d "$SDK" ] || fail "no Android SDK — set ANDROID_HOME"
export ANDROID_HOME="$SDK"
TOOLS="$SDK/build-tools/$(pin buildTools)"

if [ "${VANILLISH_SKIP_RUST:-}" != 1 ]; then
    "$ROOT/android/build-rust.sh"
fi

gradle_args=(--no-daemon)
[ -n "${VANILLISH_VERSION_NAME:-}" ] && gradle_args+=("-Pvanillish.versionName=$VANILLISH_VERSION_NAME")
[ -n "${VANILLISH_VERSION_CODE:-}" ] && gradle_args+=("-Pvanillish.versionCode=$VANILLISH_VERSION_CODE")
task="assemble$(echo "${VARIANT:0:1}" | tr '[:lower:]' '[:upper:]')${VARIANT:1}"

cd "$ROOT/android"
GRADLE=./gradlew
[ -x "$GRADLE" ] || GRADLE=gradle
rm -rf "app/build/outputs/apk/$VARIANT"
"$GRADLE" "${gradle_args[@]}" "$task"

# What Gradle made, from its own record of it.
meta="app/build/outputs/apk/$VARIANT/output-metadata.json"
[ -f "$meta" ] || fail "Gradle wrote no $meta"
read -r apk_file version_code version_name < <(python3 -c '
import json, sys
e = json.load(open(sys.argv[1]))["elements"][0]
print(e["outputFile"], e["versionCode"], e["versionName"])' "$meta")
built="app/build/outputs/apk/$VARIANT/$apk_file"
[ -f "$built" ] || fail "no $built"

# Inside: libvanillish.so for arm64, and no shared C++ runtime riding in with a dependency.
libs="$(python3 -c '
import sys, zipfile
print(" ".join(n for n in zipfile.ZipFile(sys.argv[1]).namelist() if n.startswith("lib/")))' "$built")"
case " $libs " in
    *" lib/arm64-v8a/libvanillish.so "*) ;;
    *) fail "the APK has no lib/arm64-v8a/libvanillish.so (its native libraries: ${libs:-none})" ;;
esac
case "$libs" in
    *libc++_shared.so*) fail "the APK carries libc++_shared.so ($libs): libvanillish.so links libc++ statically, and two C++ runtimes in one process is a crash waiting" ;;
esac
[ "$libs" = "lib/arm64-v8a/libvanillish.so" ] || echo "vanillish: the APK's native libraries: $libs"

# Aligned for 16 KB pages (only bites if a library is ever stored uncompressed: useLegacyPackaging).
if [ -x "$TOOLS/zipalign" ]; then
    "$TOOLS/zipalign" -c -P 16 4 "$built" || fail "the APK is not zip-aligned for 16 KB pages"
fi

# The signature, and whose it is.
[ -x "$TOOLS/apksigner" ] || fail "no apksigner in $TOOLS (sdkmanager --install \"build-tools;$(pin buildTools)\")"
certs="$("$TOOLS/apksigner" verify --print-certs "$built")" || fail "apksigner rejects $built"
cert_dn="$(printf '%s\n' "$certs" | sed -n 's/^Signer #1 certificate DN: //p' | head -n 1)"
cert_sha="$(printf '%s\n' "$certs" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p' | head -n 1)"
case "$cert_dn" in
    *"CN=Android Debug"*) signed_with=debug ;;
    *) signed_with=release ;;
esac
if [ "$signed_with" = debug ]; then
    warning="DEBUG-SIGNED: this APK is signed with this machine's debug key, not the Vanillish release key. Android installs an update only over an app signed with the same key: this APK cannot update an install of any other build (a CI run without the signing secrets makes up a new key every time), and uninstalling to get past that DELETES the app's data — the game data, the settings and the SavedVariables. Use it on a device without Vanillish, or set the VANILLISH_KEYSTORE* signing secrets."
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        echo "::warning title=Debug-signed APK::$warning"
    fi
    printf '\nvanillish: ************************************************************************\n' >&2
    printf 'vanillish: %s\n' "$warning" >&2
    printf 'vanillish: ************************************************************************\n\n' >&2
    [ "${VANILLISH_REQUIRE_RELEASE_KEY:-}" != 1 ] || fail "a release must be signed with the release key (VANILLISH_REQUIRE_RELEASE_KEY=1)"
fi

short="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
label="${VANILLISH_VERSION_NAME:-$short}"
name="Vanillish-$label$([ "$VARIANT" = debug ] && echo -debug || true).apk"
mkdir -p "$OUT"
APK="$OUT/$name"
cp "$built" "$APK"
apk_sha="$(sha256 "$APK")"
apk_size="$(du -h "$APK" | cut -f1)"
min_sdk="$(pin minSdk)"
app_id="$PACKAGE$([ "$VARIANT" = debug ] && echo .debug || true)"

if [ "$signed_with" = release ]; then
    signing="the Vanillish release key — certificate SHA-256 \`$cert_sha\`"
else
    signing="**a debug key** (\`$cert_dn\`): it cannot update an install signed with any other key"
fi
{
    echo "## Build"
    echo ""
    echo "| | |"
    echo "|---|---|"
    echo "| commit | \`$short\`${VANILLISH_CI_BUILD:+ (CI build $VANILLISH_CI_BUILD)} |"
    echo "| version | $version_name (versionCode $version_code), $VARIANT |"
    echo "| device | $(android_name "$min_sdk") (API $min_sdk) or later, arm64 — targets API $(pin targetSdk) |"
    echo "| package | \`$app_id\` |"
    echo "| size | $apk_size |"
    echo "| SHA-256 | \`$apk_sha\` |"
    echo "| signing | $signing |"
    echo ""
    if [ "$signed_with" = debug ]; then
        echo "> **Debug-signed.** Android installs an update only over an app signed with the same key, and"
        echo "> uninstalling deletes the app's data (the game data, the settings, the SavedVariables)."
        echo "> Install this build only on a device without Vanillish."
        echo ""
    fi
    echo "## Install"
    echo ""
    echo "1. On the phone, download \`$name\` (from the release, or from the CI run's artifacts)."
    echo "2. Open it. Android asks to let the browser or the Files app install unknown apps: allow it. If Play Protect warns about an unknown app: *More details* → *Install anyway*."
    echo "3. **Updating: install the new APK over the installed one — never uninstall first.** Uninstalling deletes Vanillish's data folder (\`Android/data/$app_id/files\`: the game data, the settings, the SavedVariables). An update installs only when it carries the same signing certificate as the installed app (its SHA-256 is above)."
    echo "4. From a computer instead: \`adb install -r $name\`."
    echo "5. The first launch asks for the game data: your own World of Warcraft 1.12.1 client's \`Data\` folder (docs/DATA.md)."
} >"$OUT/build-notes.md"

{
    echo "APK=$APK"
    echo "APK_NAME=$name"
    echo "VERSION_NAME=$version_name"
    echo "VERSION_CODE=$version_code"
    echo "SIGNED_WITH=$signed_with"
    echo "CERT_SHA256=$cert_sha"
    echo "COMMIT=$short"
} >"$OUT/build.env"

echo "vanillish: $name — $version_name ($version_code), $apk_size, signed with the $signed_with key${cert_sha:+ ($cert_sha)}"
ls -lh "$APK"
