#!/bin/bash
# Tests for setup-mac.sh's own logic, under any bash from macOS' 3.2 on: the path a player drags in,
# .env editing, the admin code (its letters, how compose reads it, which codes the menu takes, when
# write_env makes one, what the summary and a dry run show), finding and planning the archives,
# reading macOS' firewall and battery, the container's states, whether a start reached them, and
# (when Docker answers) the copy into a volume, whole or broken off.
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
code_ok() { printf '%s' "$1" | LC_ALL=C grep -Eqx '[A-HJ-NP-Z2-9]{4}(-[A-HJ-NP-Z2-9]{4}){3}'; }
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
# env_compose reads a value as compose hands it to the containers (as docker compose config reads
# these): compose_reads VALUE — what it makes of the line ADMIN_CODE=VALUE, CRLF.
compose_reads() {
    printf 'ADMIN_CODE=%s\r\n' "$1" >"$T/env-compose"
    ENV_FILE="$T/env-compose" env_compose ADMIN_CODE
}
tab="$(printf '\t')"
check "env_compose: an inline comment left out" WXYZ-2345-ABCD-EFGH "$(compose_reads 'WXYZ-2345-ABCD-EFGH # shared with Bob')"
check "env_compose: double quotes, a comment after" WXYZ-2345-ABCD-EFGH "$(compose_reads '"WXYZ-2345-ABCD-EFGH" # mine')"
check "env_compose: single quotes, a comment after" WXYZ-2345-ABCD-EFGH "$(compose_reads "'WXYZ-2345-ABCD-EFGH'# mine")"
check "env_compose: inside quotes, ' #' is the value" "WXYZ # 2345" "$(compose_reads '"WXYZ # 2345"')"
check "env_compose: a # with no space before it is the value" "WXYZ-2345#mine" "$(compose_reads 'WXYZ-2345#mine')"
check "env_compose: a tab, then # is the value (compose cuts at a space only)" "WXYZ-2345$tab# mine" \
    "$(compose_reads "WXYZ-2345$tab# mine")"
check "env_compose: the blanks around left out, those inside kept" "wxyz 2345 abcd" "$(compose_reads " $tab wxyz 2345 abcd $tab")"
check "env_compose: a comment alone is the value" "# off" "$(compose_reads ' # off')"
check "env_compose: a vertical tab or form feed around it dropped, the letters v, t, f and r kept" vtfr-2345-rftv \
    "$(compose_reads "$(printf ' \v\f vtfr-2345-rftv\v ')")"
# Compose's blanks beyond ASCII (compose-go's dotenv parser, as docker compose config reads these):
# U+0085 and U+00A0 before a value, Go's unicode.IsSpace after it.
nbsp="$(printf '\302\240')" nel="$(printf '\302\205')"
check "env_compose: a no-break space and a next line around it dropped" WXYZ-2345-ABCD-EFGH \
    "$(compose_reads "$nbsp$nel WXYZ-2345-ABCD-EFGH$nel$tab$nbsp")"
check "env_compose: a no-break space before ' # mine' dropped with the comment" WXYZ-2345-ABCD-EFGH \
    "$(compose_reads "WXYZ-2345-ABCD-EFGH$nbsp # mine")"
check "env_compose: an ideographic space after it dropped, an en quad before it kept" "$(printf '\342\200\200')WXYZ-2345" \
    "$(compose_reads "$(printf '\342\200\200')WXYZ-2345$(printf '\343\200\200')")"
check "env_compose: a zero-width space after it kept" "WXYZ-2345$(printf '\342\200\213')" \
    "$(compose_reads "WXYZ-2345$(printf '\342\200\213')")"
check "env_compose: empty" "" "$(compose_reads '')"
check "env_compose: absent" "" "$(env_compose NOPE)"
s="$(secret)"
check "secret: 32 hex digits" yes "$(yes_no hex32 "$s")"
check "host_tz: a tz name" yes "$(yes_no tz_name "$(host_tz)")"

# ---- the admin code ----
check "code_letters: every byte value, each of the 32 letters 8 times" "8" \
    "$(awk 'BEGIN { for (i = 0; i < 256; i++) print i }' | code_letters | fold -w1 | LC_ALL=C sort | uniq -c | awk '{ print $1 }' | sort -u)"
check "code_letters: exactly the 32 letters (no I, O, 0, 1)" 23456789ABCDEFGHJKLMNPQRSTUVWXYZ \
    "$(awk 'BEGIN { for (i = 0; i < 256; i++) print i }' | code_letters | fold -w1 | LC_ALL=C sort -u | tr -d '\n')"
check "code_letters: a byte's low 5 bits (0, 31, 32, 255)" A9A9 "$(echo '0 31 32 255' | code_letters)"
c1="$(admin_code)"
c2="$(admin_code)"
check "admin_code: ABCD-EFGH-JKLM-NPQR from the 32 letters" yes "$(yes_no code_ok "$c1")"
check "admin_code: another each time" yes "$([ "$c1" != "$c2" ] && echo yes || echo no)"
# The rule the data host, the agent and the app share: a random code, read without case, dashes or
# spaces. The labels never show a made code.
usable() { [ -z "$(code_problem "$1")" ]; }
check "code_problem: a code admin_code makes" yes "$(yes_no usable "$c1")"
for c in ABCD-EFGH-JKLM-NPQR "wxyz 2345 abcd efgh" ABCDEFGHJKLMNPQRSTUV 0123456789abcdef0123 \
    "0123 4567 89AB CDEF 0123 4567"; do
    check "code_problem: '$c' is usable" yes "$(yes_no usable "$c")"
done
for c in "" hunter2 ABCD-EFGH-JKLM-NPQ ABCDEFGHIJKLMNOP VanillishServer2024 MY-OWN-CODE-1234-5678 \
    0123456789abcdef012 CHANGE-ME-NOW-PLEASE changemeABCDEFGHJK ABCD_EFGH_JKLM_NPQR ABCD-EFGH-JKLM-NPQß; do
    check "code_problem: '$c' is not" no "$(yes_no usable "$c")"
done
check "code_problem: says which character" yes "$(yes_no has_text "$(code_problem ABCD-EFGH-JKLM-NPQß)" "a character other than")"
# ASCII whitespace as Rust's is_ascii_whitespace reads it: space, tab, LF, FF and CR, never a vertical tab.
check "code_problem: a tab, a line break, a form feed and a CR are left out" yes \
    "$(yes_no usable "$(printf 'ABCD\tEFGH\nJKLM\fNPQR\r')")"
check "code_problem: a vertical tab is not" "no yes" \
    "$(yes_no usable "$(printf 'ABCD\vEFGH-JKLM-NPQR')") $(yes_no has_text "$(code_problem "$(printf 'ABCD\vEFGH-JKLM-NPQR')")" "a character other than")"
check "code_problem: change-me, in any case" yes "$(yes_no has_text "$(code_problem Change-Me-ABCD-EFGH-JKLM)" "change-me")"
check "code_problem: a hand-picked one, and what it takes" yes \
    "$(yes_no has_text "$(code_problem VanillishServer2024)" "not a random code (the setup's 16 letters, or 20 or more hex digits from openssl rand -hex 10)")"
check "code_problem: never the code" no "$(yes_no has_text "$(code_problem VanillishServer2024)" anillish)"

# ---- write_env: the admin code (no Docker: no database volume to ask about) ----
# shellcheck disable=SC2034 # read by setup-mac.sh's functions
DIR="$T/srv" IP=192.0.2.10 DOCKER_OK=0 DRY=0 PROJECT=vanillish-setup-test
mkdir -p "$DIR"
ENV_FILE="$DIR/.env"
write_env >/dev/null 2>&1
code="$(env_get ADMIN_CODE)"
check "write_env, a fresh .env: an admin code" yes "$(yes_no code_ok "$code")"
check "write_env, a fresh .env: the summary shows that one" "$code" "$ADMIN_NEW"
check "write_env, a fresh .env: INILLA_ADMIN=lan" lan "$(env_get INILLA_ADMIN)"
check "write_env, a fresh .env: mode 600" 600 "$(stat -c %a "$ENV_FILE" 2>/dev/null || stat -f %Lp "$ENV_FILE")"
pw="$(env_get DB_PASSWORD)"
write_env >/dev/null 2>&1
check "write_env again: the code kept" "$code" "$(env_get ADMIN_CODE)"
check "write_env again: nothing new to show" "" "$ADMIN_NEW"
check "write_env again: the passwords kept" "$pw" "$(env_get DB_PASSWORD)"
grep -v '^ADMIN_CODE=' "$ENV_FILE" >"$T/env-no-code"
cat "$T/env-no-code" >"$ENV_FILE"
write_env >/dev/null 2>&1
code="$(env_get ADMIN_CODE)"
check "write_env, no ADMIN_CODE line (an older install): a code" yes "$(yes_no code_ok "$code")"
check "write_env, no ADMIN_CODE line: the summary shows it" "$code" "$ADMIN_NEW"
env_set ADMIN_CODE ""
write_env >/dev/null 2>&1
check "write_env, ADMIN_CODE= empty (menu off): kept empty" "ADMIN_CODE=" "$(grep '^ADMIN_CODE' "$ENV_FILE")"
check "write_env, ADMIN_CODE= empty: nothing to show" "" "$ADMIN_NEW"
for c in change-me-admin changeme CHANGE-ME "Change Me Now" '"chAnge-mE"' "change-me # a new one"; do
    env_set ADMIN_CODE "$c"
    write_env >/dev/null 2>&1
    code="$(env_get ADMIN_CODE)"
    check "write_env, a change-me ($c): replaced, shown" "yes yes 0" \
        "$(yes_no code_ok "$code") $([ "$ADMIN_NEW" = "$code" ] && echo yes || echo no) $ADMIN_BAD"
    check "write_env, a change-me ($c): one ADMIN_CODE line" 1 "$(grep -c '^ADMIN_CODE=' "$ENV_FILE")"
done
for c in 0123456789abcdef0123 wxyz-2345-abcd-efgh; do
    env_set ADMIN_CODE "$c"
    write_env >/dev/null 2>"$T/err"
    check "write_env, a random code of the owner's own ($c): kept, quiet" "$c 0 " "$(env_get ADMIN_CODE) $ADMIN_BAD $(cat "$T/err")"
done
env_set ADMIN_CODE MY-OWN-CODE-1234-5678
write_env >/dev/null 2>"$T/err"
check "write_env, a hand-picked code: kept, but not usable" "MY-OWN-CODE-1234-5678 1 " "$(env_get ADMIN_CODE) $ADMIN_BAD $ADMIN_NEW"
check "write_env, a hand-picked code: says the menu is off, and why" "yes yes" \
    "$(yes_no has_text "$(cat "$T/err")" "not usable, so the admin menu is off") $(yes_no has_text "$(cat "$T/err")" "not a random code")"
check "write_env, a hand-picked code: never prints it" no "$(yes_no grep -q MY-OWN "$T/err")"
# Read as compose reads it: an inline comment or the quotes are no part of the code.
for c in "wxyz-2345-abcd-efgh # shared with Bob" '"wxyz-2345-abcd-efgh" # mine' "'wxyz-2345-abcd-efgh' # mine"; do
    env_set ADMIN_CODE "$c"
    write_env >/dev/null 2>"$T/err"
    check "write_env, a code with a comment after it ($c): kept as it is, quiet" "ADMIN_CODE=$c 0 " \
        "$(grep '^ADMIN_CODE=' "$ENV_FILE") $ADMIN_BAD $ADMIN_NEW$(cat "$T/err")"
    out="$(admin_note)"
    check "admin_note, a code with a comment after it ($c): where it is" "yes no no" \
        "$(yes_no has_text "$out" "ADMIN_CODE in $DIR/.env") $(yes_no has_text "$out" "not usable") $(yes_no has_text "$out" wxyz)"
done
# A no-break space around it (a code copied from a web page or a chat): compose drops it too.
for c in "wxyz-2345-abcd-efgh$nbsp" "${nbsp}wxyz-2345-abcd-efgh" "wxyz-2345-abcd-efgh$nbsp # mine"; do
    env_set ADMIN_CODE "$c"
    write_env >/dev/null 2>"$T/err"
    check "write_env, a no-break space around a code (${c//$nbsp/<NBSP>}): kept as it is, usable, quiet" "ADMIN_CODE=$c 0 " \
        "$(grep '^ADMIN_CODE=' "$ENV_FILE") $ADMIN_BAD $ADMIN_NEW$(cat "$T/err")"
done
for c in "wxyz-2345-abcd-efgh#mine" "wxyz-2345-abcd-efgh$tab# mine"; do
    env_set ADMIN_CODE "$c"
    write_env >/dev/null 2>"$T/err"
    check "write_env, a # compose keeps ($c): not usable, and why" "1 yes" \
        "$ADMIN_BAD $(yes_no has_text "$(cat "$T/err")" "a character other than")"
done
env_set ADMIN_CODE MY-OWN-CODE-1234-5678
write_env >/dev/null 2>&1

# ---- the summary's admin line, the dry run's .env ----
out="$(admin_note)"
check "admin_note, a hand-picked code: not usable, why, and how to fix" "yes yes yes no" \
    "$(yes_no has_text "$out" "Server): ADMIN_CODE in $DIR/.env is not usable, so the admin menu is off:") $(yes_no has_text "$out" "not a random code") $(yes_no has_text "$out" "set ADMIN_CODE=change-me there and run the setup again") $(yes_no has_text "$out" MY-OWN)"
ADMIN_NEW=WXYZ-2345-ABCD-EFGH ADMIN_BAD=0 STACK_APPLIED=1 DRY=0
out="$(admin_note)"
check "admin_note: a code made now is shown" yes "$(yes_no has_text "$out" "Admin code (in Vanillish: the N button, then Settings → Server): ${B}WXYZ-2345-ABCD-EFGH")"
check "admin_note: a code made now, the stack started: live now" no "$(yes_no has_text "$out" "next run")"
STACK_APPLIED=0
out="$(admin_note)"
check "admin_note: a code made on a run that started nothing: shown, live from the next run" "yes yes" \
    "$(yes_no has_text "$out" WXYZ-2345-ABCD-EFGH) $(yes_no has_text "$out" "It takes effect after the next run of the setup, once the pathfinding is done.")"
DRY=1
out="$(admin_note)"
check "admin_note, dry run: never the code" no "$(yes_no has_text "$out" WXYZ-2345-ABCD-EFGH)"
check "admin_note, dry run: where it would be" "yes no" \
    "$(yes_no has_text "$out" "Server): ADMIN_CODE in $DIR/.env") $(yes_no has_text "$out" "next run")"
# shellcheck disable=SC2034 # DRY is read by admin_note
ADMIN_NEW="" DRY=0
env_set ADMIN_CODE 0123456789abcdef0123
out="$(admin_note)"
check "admin_note, a code kept: not shown, where it is" "no yes" \
    "$(yes_no has_text "$out" 0123456789abcdef0123) $(yes_no has_text "$out" "ADMIN_CODE in $DIR/.env")"
env_set ADMIN_CODE ""
check "admin_note, no code (menu off): nothing" "" "$(admin_note)"
printf 'DB_PASSWORD=abc\nSIGNUP_DB_PASSWORD=\nADMIN_CODE=WXYZ-2345-ABCD-EFGH\nINILLA_DATA_TOKEN=tok\nREALM_ADDRESS=1.2.3.4\nINILLA_ADMIN=lan\n' >"$T/masked"
check "masked_env: passwords, the admin code and the token hidden" \
    "DB_PASSWORD=… SIGNUP_DB_PASSWORD=… ADMIN_CODE=… INILLA_DATA_TOKEN=… REALM_ADDRESS=1.2.3.4 INILLA_ADMIN=lan" \
    "$(masked_env "$T/masked" | tr '\n' ' ' | sed 's/ $//')"
printf 'ADMIN_CODE=\nINILLA_DATA_TOKEN=\n' >"$T/masked"
check "masked_env: an empty code or token shown as it is (off, open)" "ADMIN_CODE= INILLA_DATA_TOKEN=" \
    "$(masked_env "$T/masked" | tr '\n' ' ' | sed 's/ $//')"

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

# ---- start_stack: whether this run's .env reached the containers (Docker stubbed) ----
# started STATES VMAPS UP_STATUS — STACK_APPLIED after start_stack, the containers in STATES, the
# vmaps there or not, `docker compose up` answering UP_STATUS; the pathfinding not done.
# shellcheck disable=SC2329 # the stubs are start_stack's to call
started() {
    (
        states="$1" vmaps="$2" up="$3"
        docker() { if [ "${2:-}" = up ]; then return "$up"; fi; }
        nohup() { "$@"; }
        svc_states() { printf '%s\n' "$states"; }
        earlier_up() { return 1; }
        extract_env() { printf 1; }
        has_mmaps() { return 1; }
        has_marker() { [ "$vmaps" = yes ]; }
        watch_start() { return 0; }
        start_stack >/dev/null 2>&1
        printf '%s' "$STACK_APPLIED"
    )
}
# shellcheck disable=SC2034 # read by start_stack
DRY=0 UP_PIDFILE="$T/up.pid" TMP="$T"
pf="$(printf '%s\n' "$st" | sed -e 's/starting/healthy/' -e 's/^extract|exited||0/extract|running||0/')"
down="$(printf '%s\n' "$pf" | sed 's/^mangosd|running|healthy/mangosd|exited|/')"
check "start_stack beside the pathfinding, the realm up: nothing started" 0 "$(started "$pf" yes 0)"
check "start_stack beside the pathfinding, the realm started" 1 "$(started "$down" yes 0)"
check "start_stack beside the pathfinding, its start failed" 0 "$(started "$down" yes 1)"
check "start_stack beside the pathfinding, no maps yet: nothing started" 0 "$(started "$down" no 0)"
check "start_stack, no pathfinding: the stack started" 1 "$(started "$(printf '%s\n' "$st" | sed 's/starting/healthy/')" yes 0)"
check "start_stack, dry run: nothing started" 0 "$(DRY=1 started "$pf" yes 0)"

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
