#!/usr/bin/env bash
# Vanillish: Vanillish.exe, the whole client for a Windows PC (x86-64, MSVC, the C runtime linked
# in), built, checked and zipped. Runs in Git Bash on Windows: a GitHub Windows runner
# (vanillish-windows.yml), or a PC with rustup, Visual Studio's C++ build tools, LLVM (for
# llvm-readobj) and python3.
#
#   windows/build-exe.sh                         # → target/vanillish-win-out/Vanillish-<commit>-windows-x64.zip
#   VANILLISH_FEATURES=dev windows/build-exe.sh  # benilla's dev instruments and a console: …-dev-windows-x64.zip
#
# Knobs:
#   VANILLISH_VERSION_NAME  a release's version (0.3.0): the zip's label and the exe's version
#                           resource; else the commit, and the workspace version + the commit
#   VANILLISH_CI_BUILD      a CI run number, for the notes and the version resource's 4th number
#   VANILLISH_FEATURES      dev, or empty for the player build
#   VANILLISH_PROFILE       Cargo.toml's `windows` profile, or another
#   VANILLISH_EXE           check and package this vanillish.exe (and the .pdb beside it) instead
#                           of building one
#
# After the build the exe must:
#   - be AMD64;
#   - have the GUI subsystem (no console window opens beside the game), the console one for dev;
#   - reserve 8 MB for the main thread's stack, as macOS and Linux give it (Windows' default: 1 MB);
#   - import only Windows' own DLLs, and none of the C runtime's (VCRUNTIME140.dll,
#     api-ms-win-crt-*): with the static CRT a PC needs no Visual C++ Redistributable;
#   - carry icon group 1, the version and the manifest (windows/vanillish.rc, through build.rs).
# Writes to target/vanillish-win-out/, made fresh: the zip and the folder it holds, symbols/
# (vanillish.pdb and commit.txt, to read a crash's addresses), build-notes.md (the build's facts and
# the install steps, which a release carries) and build.env (ZIP=, ZIP_NAME=, COMMIT=, SHA256= for CI).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT="$ROOT/target/vanillish-win-out"
TRIPLE=x86_64-pc-windows-msvc
PROFILE="${VANILLISH_PROFILE:-windows}"
FEATURES="${VANILLISH_FEATURES:-}"

fail() {
    echo "vanillish: $*" >&2
    exit 1
}
sha256() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
# Paths as Windows programs and the Actions runner take them (D:/a/…), and back (/d/a/…): Git
# Bash's own are the second kind. Elsewhere, unchanged.
native() {
    if command -v cygpath >/dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi
}
unix() {
    if command -v cygpath >/dev/null; then cygpath -u "$1"; else printf '%s' "$1"; fi
}

case ",${FEATURES// /,}," in
    *,dev,*) dev=1 ;;
    *) dev=0 ;;
esac

# python3 writes the zip. On Windows it may be `python` only, and `python3` the Store's stub.
PY=""
for py in python3 python; do
    if command -v "$py" >/dev/null && "$py" -c 'import sys; sys.exit(sys.version_info < (3, 8))' 2>/dev/null; then
        PY="$py"
        break
    fi
done
[ -n "$PY" ] || fail "no python3 (3.8 or later)"

# llvm-readobj reads the exe: on PATH, LLVM's installer's folder (preinstalled on GitHub's Windows
# runners), or rustup's llvm-tools.
READOBJ=""
# rustc only says where llvm-tools would be: without it (VANILLISH_EXE on a bare PC) the search
# goes on.
host="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p' | tr -d '\r')" || host=""
sysroot="$(rustc --print sysroot 2>/dev/null | tr -d '\r')" || sysroot=""
for r in "$(command -v llvm-readobj || true)" "/c/Program Files/LLVM/bin/llvm-readobj.exe" \
    "${sysroot:+$(unix "$sysroot")/lib/rustlib/$host/bin/llvm-readobj}" \
    "${sysroot:+$(unix "$sysroot")/lib/rustlib/$host/bin/llvm-readobj.exe}"; do
    if [ -n "$r" ] && [ -x "$r" ]; then
        READOBJ="$r"
        break
    fi
done
[ -n "$READOBJ" ] || fail "no llvm-readobj — install LLVM (winget install LLVM.LLVM) or rustup component add llvm-tools"

if [ -z "${VANILLISH_EXE:-}" ]; then
    case "$(uname -s)" in
        MINGW* | MSYS* | CYGWIN*) ;;
        *) fail "the MSVC build links with Visual Studio's linker: run this in Git Bash on Windows (or VANILLISH_EXE=… to check and package an exe built there)" ;;
    esac
    command -v cargo >/dev/null || fail "no Rust toolchain: install rustup (https://rustup.rs)"
    rustup target add "$TRIPLE" >/dev/null 2>&1 || true
    feature_args=()
    if [ -n "$FEATURES" ]; then
        feature_args=(--features "$FEATURES")
    fi
    echo "vanillish: building Vanillish.exe ($TRIPLE, $PROFILE${FEATURES:+, features: $FEATURES}, static CRT) with $(rustc -V)"
    # The static CRT, for this target only (build scripts stay host builds); cc-rs follows it with
    # /MT for the C. Here, not in .cargo/config.toml, which is upstream's.
    CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static" \
        cargo build --profile "$PROFILE" --target "$TRIPLE" -p vanillish-windows ${feature_args[@]+"${feature_args[@]}"}
    case "$PROFILE" in
        dev | test) profile_dir=debug ;;
        bench) profile_dir=release ;;
        *) profile_dir="$PROFILE" ;;
    esac
    EXE="$(unix "${CARGO_TARGET_DIR:-$ROOT/target}")/$TRIPLE/$profile_dir/vanillish.exe"
else
    EXE="$(unix "$VANILLISH_EXE")"
fi
[ -f "$EXE" ] || fail "no $EXE"
PDB="${EXE%.exe}.pdb"

problems=0
problem() {
    echo "vanillish: $*" >&2
    problems=$((problems + 1))
}

# 1. The machine and the subsystem, from the PE headers.
headers="$("$READOBJ" --file-headers "$(native "$EXE")" | tr -d '\r')"
machine="$(sed -n 's/^ *Machine: \([A-Z0-9_]*\).*/\1/p' <<<"$headers" | head -n 1)"
[ "$machine" = IMAGE_FILE_MACHINE_AMD64 ] || problem "the exe is for ${machine:-an unreadable machine}, not AMD64"
subsystem="$(sed -n 's/^ *Subsystem: \([A-Z0-9_]*\).*/\1/p' <<<"$headers" | head -n 1)"
if [ "$dev" = 1 ]; then want=IMAGE_SUBSYSTEM_WINDOWS_CUI; else want=IMAGE_SUBSYSTEM_WINDOWS_GUI; fi
[ "$subsystem" = "$want" ] ||
    problem "the exe's subsystem is ${subsystem:-unreadable}, not $want (src/main.rs's windows_subsystem: GUI unless the dev feature)"
# The main thread's stack, which the exe's header sets: 8 MB as on macOS and Linux (build.rs).
stack="$(sed -n 's/^ *SizeOfStackReserve: \([0-9]*\).*/\1/p' <<<"$headers" | head -n 1)"
[ "${stack:-0}" -ge 8388608 ] ||
    problem "the exe's main-thread stack reserve is ${stack:-unreadable} bytes, not 8 MB (build.rs's /STACK)"

# 2. The DLLs it imports: Windows' own, as on every Windows 10 and 11 x64, N editions included. An
# import missing here fails the build by name: widen the list on purpose, never for a DLL a PC may
# lack. (wgpu loads d3d12, dxgi, the shader compilers and vulkan-1.dll itself, at run time.)
allowed="
    KERNEL32 KERNELBASE NTDLL
    USER32 GDI32 IMM32 DWMAPI UXTHEME COMCTL32 SHELL32 SHLWAPI SHCORE
    OLE32 OLEAUT32 COMBASE PROPSYS RPCRT4
    ADVAPI32 USERENV SECUR32 CRYPT32 BCRYPT BCRYPTPRIMITIVES NCRYPT
    WS2_32 MSWSOCK IPHLPAPI
    AVRT WINMM MMDEVAPI
    PDH POWRPROF PSAPI VERSION
    UIAUTOMATIONCORE
    SETUPAPI CFGMGR32 HID XINPUT1_4
    D3D12 DXGI D3D11 D3DCOMPILER_47 OPENGL32
    DBGHELP
"
allowed=" $(printf '%s' "$allowed" | tr -s ' \n' '  ') "
imports="$("$READOBJ" --coff-imports "$(native "$EXE")" | tr -d '\r' | sed -n 's/^ *Name: //p' | sort -fu)"
[ -n "$imports" ] || problem "no imports could be read from the exe"
crt=""
for dll in $imports; do
    up="$(printf '%s' "$dll" | tr '[:lower:]' '[:upper:]')"
    case "$up" in
        VCRUNTIME*.DLL | MSVCP*.DLL | UCRTBASE*.DLL | API-MS-WIN-CRT-*.DLL | VCOMP*.DLL | CONCRT*.DLL)
            crt="$crt${crt:+, }$dll"
            ;;
        API-MS-WIN-CORE-*.DLL | API-MS-WIN-SHCORE-*.DLL) ;;
        *)
            case "$allowed" in
                *" ${up%.DLL} "*) ;;
                *) problem "imports $dll, which is not on the list of Windows' own DLLs in windows/build-exe.sh: if every Windows 10 and 11 PC has it, add it there; if not, it must ship in the zip or be loaded at run time" ;;
            esac
            ;;
    esac
done
[ -z "$crt" ] ||
    problem "imports the C runtime as DLLs ($crt), which a PC without the Visual C++ Redistributable lacks: the static CRT did not apply (a RUSTFLAGS in the environment overrides CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS)"

# 3. The resources: icon group 1 (Explorer shows an exe's first group; the window loads 1), the
# version (16) and the manifest (24).
resources="$("$READOBJ" --coff-resources "$(native "$EXE")" | tr -d '\r')"
group="$(awk '/Type: .*\(ID 14\)/ { g = 1; next } g && /Name: / { sub(/.*\(ID /, ""); sub(/\).*/, ""); print; exit }' <<<"$resources")"
[ "$group" = 1 ] || problem "the exe's first icon group is ${group:-missing}, not 1 (windows/vanillish.rc, embedded by build.rs for an MSVC target)"
grep -Eq 'Type: .*\(ID 16\)' <<<"$resources" || problem "the exe has no version resource (windows/vanillish.rc)"
grep -Eq 'Type: .*\(ID 24\)' <<<"$resources" || problem "the exe has no manifest (windows/vanillish.rc, windows/vanillish.manifest)"

# 4. The PDB, which only a build that keeps its symbols writes ([profile.windows]: strip = false).
[ -f "$PDB" ] || problem "no $(basename "$PDB") beside the exe: the profile must keep strip = false"

echo "vanillish: Vanillish.exe imports $(printf '%s\n' "$imports" | paste -sd, - | sed 's/,/, /g')"
[ "$problems" -eq 0 ] || fail "$problems problem(s) with Vanillish.exe (above)"
echo "vanillish: checked Vanillish.exe: $machine, $subsystem, main-thread stack $((stack / 1048576)) MB, Windows' own DLLs only, icon group 1, version and manifest"

short="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
workspace_version="$(awk '/^\[workspace\.package\]/ { w = 1; next } /^\[/ { w = 0 }
    w && /^version *=/ { gsub(/.*= *"|".*/, ""); print; exit }' "$ROOT/Cargo.toml" | tr -d '\r')"
version="${VANILLISH_VERSION_NAME:-${workspace_version:-0.0.0}+$short}"
label="${VANILLISH_VERSION_NAME:-$short}"
name="Vanillish-$label$([ "$dev" = 1 ] && echo -dev || true)-windows-x64.zip"

# A fresh output folder: nothing from another build reaches the zip.
rm -rf "$OUT"
mkdir -p "$OUT/Vanillish" "$OUT/symbols"
cp "$EXE" "$OUT/Vanillish/Vanillish.exe"
cp "$PDB" "$OUT/symbols/vanillish.pdb"
# Beside the exe too: a crash report's backtrace names functions only where the PDB is found, and
# the exe names it `vanillish.pdb` alone (`/PDBALTPATH`), which the exe's own folder answers.
cp "$PDB" "$OUT/Vanillish/vanillish.pdb"
echo "$short" >"$OUT/symbols/commit.txt"
cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/windows/Vanillish-512.png" "$OUT/Vanillish/"
# CRLF: Notepad's line endings.
awk '{ printf "%s\r\n", $0 }' >"$OUT/Vanillish/README.txt" <<EOF
Vanillish for Windows
=====================

Version $version, commit $short.

A World of Warcraft 1.12.1 client (benilla) for a 1.12.1 server. It needs a 1.12.1 client's
game data, which you provide: a Vanillish server offers the download, or use your own World of
Warcraft 1.12.1 folder. No Blizzard content is included; Vanillish is not affiliated with
Blizzard Entertainment.

Install
- Extract this folder somewhere you own, e.g. C:\\Games\\Vanillish. Never in Program Files
  (Windows does not let it write there) and never inside a World of Warcraft folder (Vanillish
  never writes into an install).
- Open Vanillish.exe. It is not code-signed, so Windows SmartScreen may say "Windows protected
  your PC": click "More info", then "Run anyway".
- To update, extract the new zip over this folder.

Your files
- Settings, saved variables, logs and the downloaded game data are in %LOCALAPPDATA%\\Vanillish
  (paste that into File Explorer's address bar). To keep them in this folder instead, put an
  empty file named portable.txt beside Vanillish.exe.
- The session log: benilla-config\\Diagnostics\\session-*.log there. From a Command Prompt,
  "Vanillish.exe --console" also prints it in the terminal.

More: https://github.com/klanthier/vanillish
EOF

ZIP="$OUT/$name"
"$PY" -c '
import os, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for f in sorted(os.listdir(src)):
        z.write(os.path.join(src, f), "Vanillish/" + f)' "$(native "$OUT/Vanillish")" "$(native "$ZIP")"
zip_sha="$(sha256 "$ZIP")"
zip_size="$(du -h "$ZIP" | cut -f1)"
exe_size="$(du -h "$OUT/Vanillish/Vanillish.exe" | cut -f1)"
# The version resource's four numbers, as build.rs writes them.
build="${VANILLISH_CI_BUILD:-0}"
[[ "$build" =~ ^[0-9]+$ ]] && [ "$build" -le 65535 ] || build=0
file_version="$(printf '%s' "${version%%[-+]*}" | cut -d. -f1-3).$build"

{
    echo "## Windows build"
    echo ""
    echo "| | |"
    echo "|---|---|"
    echo "| commit | \`$short\`${VANILLISH_CI_BUILD:+ (CI build $VANILLISH_CI_BUILD)} |"
    echo "| version | $version (file version $file_version), $([ "$dev" = 1 ] && echo "dev: instruments and a console" || echo player) |"
    echo "| PC | Windows 10 or 11, x64, with DirectX 12 or Vulkan graphics |"
    echo "| exe | \`Vanillish.exe\`, $exe_size: Windows' own DLLs only, no Visual C++ Redistributable |"
    echo "| zip | \`$name\`, $zip_size |"
    echo "| SHA-256 | \`$zip_sha\` (the zip) |"
    echo "| signing | none: SmartScreen asks once (below) |"
    echo ""
    echo "## Install steps"
    echo ""
    echo "1. Download \`$name\` (from the release, or from the CI run's artifacts) and extract it to a folder you own, such as \`C:\\Games\\Vanillish\`: not Program Files (Vanillish could not write there) and not inside a World of Warcraft folder (Vanillish never writes into an install)."
    echo "2. Open \`Vanillish.exe\`. It is not code-signed, so Windows SmartScreen says *Windows protected your PC*: *More info* → *Run anyway*. (Windows 11's Smart App Control, where it is on, blocks unsigned apps outright.)"
    echo "3. **Updating: extract the new zip over the same folder.** The settings, the saved variables, the logs and the game data live in \`%LOCALAPPDATA%\\Vanillish\` (or beside the exe, with a \`portable.txt\` there), never in the zip."
    echo "4. The first launch asks for the game data: connect to a Vanillish server, which offers the download, or use your own World of Warcraft 1.12.1 folder, read in place (docs/DATA.md)."
    echo "5. Something off: the session log is \`benilla-config\\Diagnostics\\session-*.log\` in that folder, and \`Vanillish.exe --console\` run from a Command Prompt prints it there too."
} >"$OUT/build-notes.md"

{
    echo "ZIP=$(native "$ZIP")"
    echo "ZIP_NAME=$name"
    echo "COMMIT=$short"
    echo "SHA256=$zip_sha"
} >"$OUT/build.env"

echo "vanillish: $name — $version, Vanillish.exe $exe_size, zip $zip_size, SHA-256 $zip_sha"
ls -lh "$ZIP" "$OUT/symbols/vanillish.pdb"
