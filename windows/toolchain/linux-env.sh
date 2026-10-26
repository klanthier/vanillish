# shellcheck shell=bash
# Vanillish: type-check the Windows build of the client on Linux, without Microsoft's SDK.
#
#   source windows/toolchain/linux-env.sh
#   cargo check --target x86_64-pc-windows-gnu -p vanillish-windows
#   cargo clippy --target x86_64-pc-windows-gnu -p inilla-platform -p vanillish-windows --no-deps -- -D warnings
#
# A check compiles every `cfg(windows)` path and links nothing, so Rust needs only rustup's prebuilt
# std for the target. What does need Windows headers is the C in the graph, which build scripts
# compile even for a check (Lua, ring's C; ring's x86-64 assembly comes pregenerated as COFF
# objects). Zig's `zig cc` compiles it against the mingw-w64 headers Zig distributes (public domain
# and ZPL), from the same pinned wheel as the iOS sysroot's Darwin headers.
#
# gnu, not msvc: the cfg(windows) code is the same for both. The shipping build is msvc, with the
# static CRT, on a Windows runner (windows/build-exe.sh, vanillish-windows.yml).

_vw_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
_vw_dir="${VANILLISH_WIN_TOOLCHAIN:-$_vw_root/target/vanillish-win-toolchain}"

# Pinned: the headers are part of the build's inputs, so they move only with this line. They must
# agree with ios/toolchain/linux-env.sh's, whose wheel this is: one download serves both.
_vw_zig_ver="0.16.0"
_vw_zig_whl="ziglang-${_vw_zig_ver}-py3-none-manylinux_2_12_x86_64.manylinux2010_x86_64.musllinux_1_1_x86_64.whl"
_vw_zig_url="https://files.pythonhosted.org/packages/3e/ed/7b79023aa27ceb5d461ecf761181e7c33c57bbc1a6256a39535d1c7083d2/${_vw_zig_whl}"
_vw_zig_sha="9fcda73f62b851dd72a54b710ad40a209896db14cfb13649e62191243556342b"

grep -qF "$_vw_zig_sha" "$_vw_root/ios/toolchain/linux-env.sh" 2>/dev/null ||
    echo "vanillish: the Zig pin here and in ios/toolchain/linux-env.sh differ: move them together" >&2

_vw_make_toolchain() {
    local work="$_vw_dir/.work"
    local ios_whl="$_vw_root/target/inilla-ios-sysroot/.work/$_vw_zig_whl"
    mkdir -p "$work"
    if [ ! -f "$work/$_vw_zig_whl" ] && [ -f "$ios_whl" ]; then
        cp "$ios_whl" "$work/$_vw_zig_whl" || return 1
    fi
    if [ ! -f "$work/$_vw_zig_whl" ]; then
        echo "vanillish: fetching Zig and the mingw-w64 headers (ziglang ${_vw_zig_ver} wheel)…" >&2
        curl -fsSL -o "$work/$_vw_zig_whl.part" "$_vw_zig_url" || return 1
        mv "$work/$_vw_zig_whl.part" "$work/$_vw_zig_whl"
    fi
    echo "$_vw_zig_sha  $work/$_vw_zig_whl" | sha256sum -c --quiet - || {
        echo "vanillish: ziglang wheel checksum mismatch" >&2
        rm -f "$work/$_vw_zig_whl"
        return 1
    }
    rm -rf "$_vw_dir/ziglang" "$_vw_dir/zigcc" "$_vw_dir/zigar" "$_vw_dir/.pin" "$work/unpacked"
    python3 -m zipfile -e "$work/$_vw_zig_whl" "$work/unpacked" || return 1
    mv "$work/unpacked/ziglang" "$_vw_dir/ziglang"
    chmod +x "$_vw_dir/ziglang/zig"
    rm -rf "$work"
    # cc-rs passes rustc's triple (--target=x86_64-pc-windows-gnu), which zig does not parse, and
    # -m64, which the zig target already says: both dropped.
    cat >"$_vw_dir/zigcc" <<EOF
#!/usr/bin/env bash
args=()
for a in "\$@"; do case "\$a" in --target=* | -m64) ;; *) args+=("\$a") ;; esac; done
exec "$_vw_dir/ziglang/zig" cc -target x86_64-windows-gnu "\${args[@]}"
EOF
    cat >"$_vw_dir/zigar" <<EOF
#!/bin/sh
exec "$_vw_dir/ziglang/zig" ar "\$@"
EOF
    chmod +x "$_vw_dir/zigcc" "$_vw_dir/zigar"
    echo "$_vw_zig_sha" >"$_vw_dir/.pin"
    echo "vanillish: Windows cross toolchain ready at $_vw_dir" >&2
}

if [ "$(cat "$_vw_dir/.pin" 2>/dev/null)" != "$_vw_zig_sha" ] || [ ! -x "$_vw_dir/zigcc" ]; then
    _vw_make_toolchain || echo "vanillish: Windows cross toolchain setup FAILED" >&2
fi

# rust-toolchain.toml's release, so the target lands beside the toolchain cargo runs here.
if ! (cd "$_vw_root" && rustup target list --installed | grep -qx x86_64-pc-windows-gnu); then
    (cd "$_vw_root" && rustup target add x86_64-pc-windows-gnu) || echo "vanillish: rustup target add x86_64-pc-windows-gnu FAILED" >&2
fi

export CC_x86_64_pc_windows_gnu="$_vw_dir/zigcc"
export AR_x86_64_pc_windows_gnu="$_vw_dir/zigar"
