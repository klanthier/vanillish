#!/bin/bash
# Tests for setup-mac.sh's own logic, under any bash from macOS' 3.2 on: the path a player drags in,
# .env editing, finding and planning the archives, reading macOS' firewall and battery, the
# container's states, and (when Docker answers) the copy into a volume, whole or broken off.
#
#   bash ci/setup-test.sh            # from server/; VANILLISH_HELPER_IMAGE picks the copy's image
#
# It sources setup-mac.sh with VANILLISH_SETUP_LIB=1, which defines the functions and runs nothing.
here="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck disable=SC2034 # read by the file sourced next
VANILLISH_SETUP_LIB=1
# (Not followed by shellcheck, which takes the script's final `exit` for the end of this one too.)
# shellcheck source=/dev/null
. "$here/setup-mac.sh"
set +e

FAILS=0
T="$(mktemp -d "${TMPDIR:-/tmp}/vanillish-setup-test.XXXXXX")"
trap 'rm -rf "$T"' EXIT
check() { # check NAME EXPECTED ACTUAL
    if [ "$2" = "$3" ]; then
        printf 'ok    %s\n' "$1"
    else
        printf 'FAIL  %s\n      expected: %s\n      got:      %s\n' "$1" "$2" "$3"
        FAILS=$((FAILS + 1))
    fi
}
yes_no() { if "$@"; then echo yes; else echo no; fi; }
# (bash 3.2 misreads a case pattern's ")" inside "$(...)", so the tests keep case out of it.)
hex32() {
    case "$1" in *[!0-9a-f]*) return 1 ;; esac
    [ "${#1}" = 32 ]
}
tz_name() { case "$1" in '' | /* | *[!A-Za-z0-9_+/-]*) return 1 ;; esac; }
has_text() { case "$1" in *"$2"*) return 0 ;; esac; return 1; }

# ---- a dragged or typed path ----
HOME=/Users/kevin
check "clean_path: Terminal's escaped drag, trailing space" "/Users/kevin/World of Warcraft" "$(clean_path '/Users/kevin/World\ of\ Warcraft ')"
check "clean_path: single quotes" "/Users/kevin/World of Warcraft" "$(clean_path "'/Users/kevin/World of Warcraft'")"
check "clean_path: double quotes keep backslashes" '/a\b c' "$(clean_path '"/a\b c"')"
# shellcheck disable=SC2088 # a typed ~, as the player types it
check "clean_path: ~/" "/Users/kevin/Games/WoW 1.12" "$(clean_path '~/Games/WoW\ 1.12')"
check "clean_path: ~" "/Users/kevin" "$(clean_path '~')"
check "clean_path: a plain path" "/Volumes/Ext/WoW" "$(clean_path '/Volumes/Ext/WoW')"
HOME="$T/home"
mkdir -p "$HOME"

# ---- addresses ----
for a in 10.0.0.5 172.16.0.1 172.31.255.1 192.168.1.20; do
    check "is_private_v4 $a" yes "$(yes_no is_private_v4 "$a")"
done
for a in 172.32.0.1 172.15.0.1 8.8.8.8 100.101.102.103 ""; do
    check "is_private_v4 '$a'" no "$(yes_no is_private_v4 "$a")"
done

# ---- .env ----
ENV_FILE="$T/env"
printf 'DB_PASSWORD="quoted"\r\nREALM_ADDRESS=1.2.3.4\nREALM_ADDRESS=5.6.7.8\n#TZ=Europe/Paris\nWOW_DATA_DIR=/nope\n' >"$ENV_FILE"
chmod 600 "$ENV_FILE"
check "env_get: quotes and CR removed" quoted "$(env_get DB_PASSWORD)"
check "env_get: the last line wins, as compose reads it" 5.6.7.8 "$(env_get REALM_ADDRESS)"
check "env_get: a comment is no value" "" "$(env_get TZ)"
check "env_get: absent" "" "$(env_get NOPE)"
env_set REALM_ADDRESS 192.168.1.20
check "env_set: one line left" "REALM_ADDRESS=192.168.1.20" "$(grep '^REALM_ADDRESS=' "$ENV_FILE")"
env_set SIGNUP_DB_PASSWORD abc
check "env_set: appended" abc "$(env_get SIGNUP_DB_PASSWORD)"
env_comment WOW_DATA_DIR
check "env_comment" "# WOW_DATA_DIR=/nope" "$(grep 'WOW_DATA_DIR' "$ENV_FILE")"
check "env_set keeps the file's mode" 600 "$(stat -c %a "$ENV_FILE" 2>/dev/null || stat -f %Lp "$ENV_FILE")"
s="$(secret)"
check "secret: 32 hex digits" yes "$(yes_no hex32 "$s")"
check "host_tz: a tz name" yes "$(yes_no tz_name "$(host_tz)")"

# ---- the game folder ----
W="$T/World of Warcraft"
D="$W/Data"
mkdir -p "$D/Interface/FrameXML" "$W/WTF"
i=1
for f in DBC.MPQ fonts.MPQ interface.MPQ misc.MPQ model.MPQ Patch.MPQ patch-2.MPQ sound.MPQ speech.MPQ \
    terrain.MPQ texture.MPQ wmo.MPQ Speech2.mpq base.MPQ backup.MPQ; do
    head -c $((i * 1000 + 7)) /dev/zero >"$D/$f"
    i=$((i + 1))
done
printf 'SET realmlist "logon.example"\r\nSET locale "enUS"\r\n' >"$W/WTF/Config.wtf"
check "entry: any case" DBC.MPQ "$(entry "$D" dbc.mpq)"
check "entry: none" no "$(yes_no entry "$D" common.MPQ)"
check "find_data: the WoW folder" "$D" "$(find_data "$W")"
check "find_data: the Data folder" "$D" "$(find_data "$D")"
check "find_data: the folder above" "$D" "$(find_data "$T")"
check "client_locale: enUS (CRLF)" enUS "$(client_locale "$D")"
plan_import "$D"
check "plan_import: 13 archives" 13 "$COUNT"
check "plan_import: the bytes" "$(cat "$D"/[!bB]*.[Mm][Pp][Qq] | wc -c | tr -d ' ')" "$TOTAL"
check "plan_import: stock names, repack extras left out" \
    "dbc.MPQ fonts.MPQ interface.MPQ misc.MPQ model.MPQ patch-2.MPQ patch.MPQ sound.MPQ speech.MPQ speech2.MPQ terrain.MPQ texture.MPQ wmo.MPQ" \
    "$(plan_list | awk '{ print $1 }' | tr '\n' ' ' | sed 's/ $//')"
check "plan_import: as found, then stock" "Speech2.mpq speech2.MPQ 13007" "$(printf '%s' "$PLAN" | grep -i speech2)"
check "has_required: the plan" yes "$(yes_no has_required "$(plan_list)")"
check "has_required: one missing" no "$(yes_no has_required "$(plan_list | grep -v '^wmo')")"
rm "$D/wmo.MPQ"
check "plan_import: a missing archive stops" 1 "$( (plan_import "$D" >/dev/null 2>&1) && echo 0 || echo 1)"
out="$( (plan_import "$D") 2>&1)"
check "plan_import: and says which" yes "$(yes_no has_text "$out" "missing: wmo.MPQ")"
head -c 2000 /dev/zero >"$D/wmo.MPQ"
printf 'SET locale "deDE"\n' >"$W/WTF/Config.wtf"
check "client_locale: deDE" deDE "$(client_locale "$D")"

# ---- macOS' answers ----
check "firewall: block all, recent wording" yes "$(yes_no firewall_blocks_all 'Firewall is enabled. (State = 1)' 'Firewall has block all state set to enabled.')"
check "firewall: block all, old wording" yes "$(yes_no firewall_blocks_all 'Firewall is enabled. (State = 1)' 'Block all ENABLED! ')"
check "firewall: state 2" yes "$(yes_no firewall_blocks_all 'Firewall is blocking all non-essential incoming connections. (State = 2)' '')"
check "firewall: on, block all off" no "$(yes_no firewall_blocks_all 'Firewall is enabled. (State = 1)' 'Firewall has block all state set to disabled.')"
check "firewall: old wording, off" no "$(yes_no firewall_blocks_all 'Firewall is enabled. (State = 1)' 'Block all DISABLED! ')"
check "firewall: off altogether" no "$(yes_no firewall_blocks_all 'Firewall is disabled. (State = 0)' 'Firewall has block all state set to enabled.')"
check "firewall: no answer" no "$(yes_no firewall_blocks_all '' '')"
batt="Now drawing from 'AC Power'
 -InternalBattery-0 (id=21823587)	100%; charged; 0:00 remaining present: true"
check "is_laptop: an M4 MacBook (Mac16,1)" yes "$(yes_no is_laptop "$batt" Mac16,1)"
check "is_laptop: an M1 MacBook Air" yes "$(yes_no is_laptop "" MacBookAir10,1)"
check "is_laptop: a Mac mini" no "$(yes_no is_laptop "Now drawing from 'AC Power'" Mac16,10)"

# ---- the containers' states ----
st='db|running|healthy|0
db-init|exited||0
extract|exited||0
realmd|running|healthy|0
mangosd|running|starting|0
datahost|running|healthy|0'
check "svc_field" starting "$(svc_field "$st" mangosd 3)"
check "all_up: mangosd still loading" no "$(yes_no all_up "$st")"
check "broken: nothing yet" no "$(yes_no broken "$st")"
check "all_up" yes "$(yes_no all_up "$(printf '%s\n' "$st" | sed 's/starting/healthy/')")"
check "broken: extract failed" yes "$(yes_no broken "$(printf '%s\n' "$st" | sed 's/^extract|exited||0/extract|exited||1/')")"
check "broken: mangosd unhealthy" yes "$(yes_no broken "$(printf '%s\n' "$st" | sed 's/starting/unhealthy/')")"

# ---- the copy into a volume (Docker) ----
if docker info >/dev/null 2>&1 && docker image inspect "$HELPER" >/dev/null 2>&1; then
    vol="vanillish-setup-test-$$"
    docker volume create "$vol" >/dev/null
    # send ARCHIVE… — stream these files of $D under the current PLAN; prints what the volume holds.
    send() {
        local renames
        renames="$(printf '%s' "$PLAN" | awk 'NF { print $1, $2, $3 }')"
        # shellcheck disable=SC2068 # the names split on purpose
        tar -cf - -C "$D" $@ | docker run --rm -i -v "$vol":/d -e RENAMES="$renames" "$HELPER" sh -euc "$IMPORT_SH" |
            LC_ALL=C sort
    }
    in_volume() {
        docker run --rm -v "$vol":/d:ro "$HELPER" sh -c 'cd /d && for f in * .[!.]*; do [ -e "$f" ] && echo "$f $(stat -c "%s %a" "$f")"; done; true' |
            LC_ALL=C sort
    }
    plan_import "$D"
    all="$(printf '%s' "$PLAN" | awk 'NF { print $1 }')"
    # shellcheck disable=SC2086 # the names split on purpose
    got="$(send $all)"
    check "copy: every archive, stock names, sizes" "$(plan_list)" "$got"
    check "copy: read-only, nothing else left" "$(plan_list | sed 's/$/ 444/')" "$(in_volume)"
    before="$(in_volume)"
    # A stream that lacks one archive (tar skips a file it cannot read) must not touch the copy.
    # shellcheck disable=SC2046,SC2086 # the names split on purpose
    send $(printf '%s\n' $all | grep -v '^model') >/dev/null 2>"$T/err"
    check "copy: a missing archive fails" yes "$(grep -q 'model.MPQ arrived with 0 of' "$T/err" && echo yes || echo no)"
    check "copy: the archives already there are kept" "$before" "$(in_volume)"
    # A stream cut short.
    # shellcheck disable=SC2086
    tar -cf - -C "$D" $all | head -c 20000 |
        docker run --rm -i -v "$vol":/d -e RENAMES="$(printf '%s' "$PLAN" | awk 'NF { print $1, $2, $3 }')" "$HELPER" sh -euc "$IMPORT_SH" >/dev/null 2>&1
    check "copy: a stream cut short keeps them too" "$before" "$(in_volume)"
    docker volume rm "$vol" >/dev/null
else
    echo "skip  the copy into a volume (no Docker, or no $HELPER image: set VANILLISH_HELPER_IMAGE)"
fi

echo
if [ "$FAILS" = 0 ]; then echo "all passed"; else echo "$FAILS failed"; fi
[ "$FAILS" = 0 ]
