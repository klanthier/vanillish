#!/usr/bin/env bash
# pins.sh — the images compose.yaml pulls, read from it and checked, as key=value lines (for
# $GITHUB_OUTPUT, or `eval`):
#
#   registry          ghcr.io/klanthier            (VANILLISH_REGISTRY's default)
#   vmangos_commit    the 40-hex VMANGOS_COMMIT default
#   vmangos_tag       <vmangos_commit>-rN          (the tag compose.yaml pulls)
#   vmangos_inputs    what the vmangos image is built from: docker/'s git tree, plus a pinned dump
#   datahost_tag      datahost/Cargo.toml's version, which both compose files must name
#   datahost_inputs   datahost/'s git tree
#   db_url, db_sha256 compose.yaml's VMANGOS_DB_URL / VMANGOS_DB_SHA256 defaults (usually empty)
#
# The inputs travel as the io.github.klanthier.vanillish.inputs label: a published tag whose label
# differs was built from other files (image-state.sh), and needs a new tag rather than a rebuild.
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
    echo "pins.sh: $*" >&2
    exit 1
}
# Exactly one distinct, non-empty line on stdin.
one() { sort -u | awk 'NF { n++; v = $0 } END { if (n != 1) exit 1; print v }'; }

registry="$(grep -oh 'VANILLISH_REGISTRY:-[^}]*' compose.yaml compose.datahost.yaml | sed 's/^[^-]*-//' | one)" ||
    fail "compose files must default VANILLISH_REGISTRY to one registry"
commit="$(grep -o 'VMANGOS_COMMIT:-[0-9a-f]*' compose.yaml | cut -d- -f2 | one)" ||
    fail "compose.yaml must default VMANGOS_COMMIT to one value"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || fail "compose.yaml's VMANGOS_COMMIT default is not a full commit: $commit"
# shellcheck disable=SC2016 # compose's own ${…}, matched literally
rev="$(sed -n 's|^ *image: .*/vanillish-vmangos:${VMANGOS_COMMIT:-[0-9a-f]*}-\(r[0-9][0-9]*\)$|\1|p' compose.yaml | one)" ||
    fail "compose.yaml's vmangos image must be …/vanillish-vmangos:\${VMANGOS_COMMIT:-<commit>}-r<N>"
datahost="$(sed -n 's|^ *image: .*/vanillish-datahost:\([0-9A-Za-z.+-]*\)$|\1|p' compose.yaml compose.datahost.yaml | one)" ||
    fail "compose.yaml and compose.datahost.yaml must name one …/vanillish-datahost:<version>"
cargo="$(sed -n 's/^version = "\(.*\)"$/\1/p' datahost/Cargo.toml | head -1)"
[ "$datahost" = "$cargo" ] ||
    fail "the compose files pull vanillish-datahost:$datahost, but datahost/Cargo.toml is version $cargo"
db_url="$(sed -n 's/.*VMANGOS_DB_URL:-\([^}]*\)}.*/\1/p' compose.yaml | sort -u)"
db_sha256="$(sed -n 's/.*VMANGOS_DB_SHA256:-\([^}]*\)}.*/\1/p' compose.yaml | sort -u)"
[ "$(printf '%s' "$db_url" | grep -c '')" -le 1 ] && [ "$(printf '%s' "$db_sha256" | grep -c '')" -le 1 ] ||
    fail "compose.yaml defaults VMANGOS_DB_URL or VMANGOS_DB_SHA256 more than once"

# Committed trees, so the same commit gives the same inputs on every runner.
docker_tree="$(git rev-parse HEAD:./docker)"
datahost_tree="$(git rev-parse HEAD:./datahost)"
vmangos_inputs="docker=$docker_tree"
[ -z "$db_url$db_sha256" ] || vmangos_inputs="$vmangos_inputs db=${db_sha256:-$db_url}"

cat <<EOF
registry=$registry
vmangos_commit=$commit
vmangos_tag=$commit-$rev
vmangos_inputs=$vmangos_inputs
datahost_tag=$datahost
datahost_inputs=datahost=$datahost_tree
db_url=$db_url
db_sha256=$db_sha256
EOF
