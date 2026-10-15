#!/usr/bin/env bash
# test_ci_tools.sh — image-state.sh and pins.sh on made-up registry answers and compose files. Their
# verdicts decide what server-images.yml publishes: a wrong "absent" overwrites a published tag, a
# wrong pin makes every Mac pull a tag that does not exist. No network and no Docker: a stub answers
# for `docker buildx imagetools inspect`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0
pass() { echo "  ok  $*"; }
bad() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

# ---- image-state.sh -----------------------------------------------------------------------------
mkdir -p "$tmp/bin"
cat >"$tmp/bin/docker" <<'EOF'
#!/bin/sh
printf '%s' "$STUB_OUT"
exit "${STUB_EXIT:-0}"
EOF
chmod +x "$tmp/bin/docker"

# One platform's image config, labelled with inputs $1 (none when empty).
config() {
    if [ -n "$1" ]; then
        printf '{"architecture":"%s","os":"linux","config":{"Labels":{"io.github.klanthier.vanillish.inputs":"%s"}}}' "${2:-amd64}" "$1"
    else
        printf '{"architecture":"%s","os":"linux","config":{}}' "${2:-amd64}"
    fi
}
# state WHAT WANT EXIT OUTPUT: image-state.sh's verdict on "docker=abc" when the registry says OUTPUT.
state() {
    local got
    got="$(STUB_EXIT="$3" STUB_OUT="$4" PATH="$tmp/bin:$PATH" "$here/image-state.sh" ghcr.io/o/i:t docker=abc 2>/dev/null)" ||
        got=error
    if [ "$got" = "$2" ]; then pass "image-state: $1 → $2"; else bad "image-state: $1: want $2, got $got"; fi
}
echo "== image-state.sh"
state "both platforms built from these files" fresh 0 \
    "{\"linux/amd64\":$(config docker=abc),\"linux/arm64\":$(config docker=abc arm64)}"
state "a third platform beside them" fresh 0 \
    "{\"linux/amd64\":$(config docker=abc),\"linux/arm64\":$(config docker=abc arm64),\"linux/riscv64\":$(config '' riscv64)}"
state "arm64 built from other files" stale 0 \
    "{\"linux/amd64\":$(config docker=abc),\"linux/arm64\":$(config docker=old arm64)}"
state "no inputs label (published by hand)" stale 0 \
    "{\"linux/amd64\":$(config ''),\"linux/arm64\":$(config '' arm64)}"
state "arm64 missing" stale 0 "{\"linux/amd64\":$(config docker=abc)}"
state "one platform, not a list" stale 0 "$(config docker=abc)"
state "no such tag" absent 1 "ERROR: ghcr.io/o/i:t: not found"
state "a package anonymous may not read" absent 1 \
    "ERROR: unexpected status from HEAD request to https://ghcr.io/v2/o/i/manifests/t: 401 Unauthorized"
state "denied" absent 1 "ERROR: denied: requested access to the resource is denied"
state "the registry unreachable" error 1 \
    "ERROR: failed to do request: Head \"https://ghcr.io/v2/o/i/manifests/t\": dial tcp: lookup ghcr.io: i/o timeout"

# ---- pins.sh ------------------------------------------------------------------------------------
# A copy of the server's pins in a repository of its own, the datahost's version made to agree.
echo "== pins.sh"
base="$tmp/base"
mkdir -p "$base/ci" "$base/datahost" "$base/docker/vmangos"
cp "$here/../compose.yaml" "$here/../compose.datahost.yaml" "$base/"
cp "$here/pins.sh" "$base/ci/"
cp "$here/../datahost/Cargo.toml" "$base/datahost/"
echo "FROM scratch" >"$base/docker/vmangos/Dockerfile"
tag="$(sed -n 's|^ *image: .*/vanillish-datahost:\(.*\)$|\1|p' "$base/compose.yaml" | head -1)"
sed -i.orig "s/^version = \".*\"$/version = \"$tag\"/" "$base/datahost/Cargo.toml"
rm "$base/datahost/Cargo.toml.orig"
git -C "$base" init -q
git -C "$base" add -A
git -C "$base" -c user.name=ci -c user.email=ci@localhost -c commit.gpgsign=false commit -q -m pins

# pins WHAT WANT-IN-OUTPUT [SED-EXPR FILE]: pins.sh on the base, FILE edited by SED-EXPR first.
pins() {
    local what="$1" want="$2" dir="$tmp/case" out
    rm -rf "$dir"
    cp -R "$base" "$dir"
    if [ $# -ge 4 ]; then
        sed -i.orig "$3" "$dir/$4"
        rm "$dir/$4.orig"
    fi
    out="$("$dir/ci/pins.sh" 2>&1)" || true
    case "$out" in
        *"$want"*) pass "pins: $what" ;;
        *) bad "pins: $what: want '$want' in: $out" ;;
    esac
}
commit="$(sed -n 's/.*VMANGOS_COMMIT:-\([0-9a-f]*\)}.*/\1/p' "$base/compose.yaml" | head -1)"
pins "the pins agree" "vmangos_tag=$commit-r"
pins "the datahost tag is Cargo.toml's version" "datahost_tag=$tag"
pins "the image inputs are the committed trees" "datahost_inputs=datahost=$(git -C "$base" rev-parse HEAD:datahost)"
pins "a datahost tag that is not Cargo.toml's version" "but datahost/Cargo.toml is version" \
    's/^version = ".*"$/version = "0.0.0-other"/' datahost/Cargo.toml
pins "the two compose files on other datahost tags" "must name one" \
    "s|/vanillish-datahost:.*$|/vanillish-datahost:0.0.0-other|" compose.datahost.yaml
pins "the two compose files on other registries" "one registry" \
    "s|VANILLISH_REGISTRY:-[^}]*|VANILLISH_REGISTRY:-ghcr.io/someone-else|" compose.datahost.yaml
pins "a short VMANGOS_COMMIT" "not a full commit" "s/VMANGOS_COMMIT:-\([0-9a-f]\{9\}\)[0-9a-f]*/VMANGOS_COMMIT:-\1/g" compose.yaml
pins "two VMANGOS_COMMIT defaults" "one value" \
    "s/VMANGOS_COMMIT: \${VMANGOS_COMMIT:-[0-9a-f]*}/VMANGOS_COMMIT: \${VMANGOS_COMMIT:-$(printf '%040d' 0)}/" compose.yaml
pins "a vmangos image without -rN" "-r<N>" 's|\(vanillish-vmangos:.*}\)-r[0-9]*$|\1|' compose.yaml

[ "$failures" = 0 ] || {
    echo "$failures check(s) failed" >&2
    exit 1
}
echo "ci tools: all checks passed"
