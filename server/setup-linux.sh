#!/bin/bash
# The Vanillish server on Linux, or on Windows inside WSL: one command, and safe to run again.
#
#   bash -c "$(curl -fsSL https://raw.githubusercontent.com/klanthier/vanillish/klanthier/inilla-android-port/server/setup-linux.sh)"
#
# The setup itself is setup-mac.sh, which carries the Linux and WSL path too, so the two never
# drift: this hands over to the copy beside it, or to the one on GitHub. Its options work here too
# (--help lists them).
set -eu

here=""
case "$0" in
    *setup-linux.sh) if [ -f "$0" ]; then here="$(cd "$(dirname "$0")" && pwd)"; fi ;;
esac
if [ -n "$here" ] && [ -f "$here/setup-mac.sh" ]; then
    exec bash "$here/setup-mac.sh" "$@"
fi

repo="${VANILLISH_REPO:-klanthier/vanillish}"
ref="${VANILLISH_REF:-klanthier/inilla-android-port}"
script="$(curl -fsSL "https://raw.githubusercontent.com/$repo/$ref/server/setup-mac.sh")" || {
    echo "could not download the setup from github.com/$repo ($ref): is this machine online?" >&2
    exit 1
}
exec bash -c "$script" vanillish-setup "$@"
