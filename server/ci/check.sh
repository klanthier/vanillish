#!/usr/bin/env bash
# check.sh [--no-datahost] — the server with no game data at all, as CI checks it on amd64 and arm64.
#
# compose.yaml's db, db-init, realmd and data host come up on an empty Data folder (mangosd and
# extract need the player's archives, so they stay off), then:
#
#   1. db-init's sign-up user can add an account the way the data host does, and nothing else
#      (no UPDATE, DELETE, v, account_access, other tables or databases); a second db-init run keeps
#      it as it is, and a stray grant is taken away by the next one;
#   2. that SQL-made account logs in to realmd with SRP6, and the realm list holds the realm;
#   3. accounts_e2e.py open: /server.json, POST /accounts (created, replay, taken, invalid, rate
#      limit), then an SRP6 logon with the account the data host made;
#   4. accounts_e2e.py token and closed: the data host again with INILLA_DATA_TOKEN set, then with
#      INILLA_SIGNUP=off;
#   5. admin_e2e.py on and off: the admin menu's door with ADMIN_CODE set (server.json's admin
#      section, nonces, the MAC: missing, wrong, replayed, right, which is 503 with no mangosd), then
#      with INILLA_ADMIN=off.
#
# Before it starts anything, compose.yaml with an ADMIN_CODE: mangosd and the data host get it,
# mangosd keeps the database settings beside it, and no other service gets it.
#
# --no-datahost stops after 2 (for a data host without /server.json yet). The images are whatever
# compose.yaml names: pulled, already here, or built. It runs as its own compose project
# (vanillish-ci) with its own env file, so a stack of yours and its volumes stay untouched; it removes
# its own when done (KEEP=1 leaves them up). Ports: CHECK_REALMD_PORT, CHECK_DATAHOST_PORT
# (3724, 8099), on 127.0.0.1.
set -euo pipefail
cd "$(dirname "$0")/.."

datahost=1
case "${1:-}" in
    --no-datahost) datahost=0 ;;
    '') ;;
    *) echo "usage: $0 [--no-datahost]" >&2; exit 2 ;;
esac

project=vanillish-ci
realm=Vanillish
realmd_port="${CHECK_REALMD_PORT:-3724}"
datahost_port="${CHECK_DATAHOST_PORT:-8099}"
world="127.0.0.1:8085"
base="http://127.0.0.1:$datahost_port/"
token="ci-token-$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
secret() { python3 -c 'import secrets; print(secrets.token_hex(16))'; }
signup_password="$(secret)"
# The admin code as the setup makes it (setup-mac.sh's admin_code): 16 bytes of /dev/urandom, each
# masked to 5 bits, one of 32 letters each, in fours.
admin_code() {
    local c
    c="$(od -An -N16 -tu1 /dev/urandom |
        awk -v a=ABCDEFGHJKLMNPQRSTUVWXYZ23456789 '{ for (i = 1; i <= NF; i++) printf "%s", substr(a, $i % 32 + 1, 1) }')"
    printf '%s-%s-%s-%s\n' "${c:0:4}" "${c:4:4}" "${c:8:4}" "${c:12:4}"
}
admin="$(admin_code)"
[[ "$admin" =~ ^[A-HJ-NP-Z2-9]{4}(-[A-HJ-NP-Z2-9]{4}){3}$ ]] || { echo "FAIL: no admin code made" >&2; exit 1; }

envfile="$(mktemp)"
cat >"$envfile" <<EOF
REALM_ADDRESS=127.0.0.1
REALM_NAME=$realm
DB_ROOT_PASSWORD=$(secret)
DB_PASSWORD=$(secret)
SIGNUP_DB_PASSWORD=$signup_password
WOW_DATA_DIR=./docker/empty
BIND_ADDR=127.0.0.1
REALMD_PORT=$realmd_port
DATAHOST_PORT=$datahost_port
INILLA_SIGNUP=lan
INILLA_DATA_TOKEN=
ADMIN_CODE=$admin
INILLA_ADMIN=lan
EOF

compose() { docker compose -p "$project" --env-file "$envfile" "$@"; }
step() { printf '\n== %s\n' "$*"; }
fail() {
    echo "FAIL: $*" >&2
    exit 1
}

finish() {
    local status=$?
    if [ "$status" != 0 ]; then
        echo "---- logs (exit $status) ----"
        compose logs --no-color --tail 80 db-init realmd datahost 2>&1 || true
    fi
    if [ "${KEEP:-0}" = 1 ]; then
        echo "KEEP=1: the stack stays up: docker compose -p $project --env-file $envfile …"
    else
        compose down -v --remove-orphans >/dev/null 2>&1 || true
        rm -f "$envfile"
    fi
    exit "$status"
}
trap finish EXIT

# As the sign-up user, against realmd's database. Prints the client's output, errors included.
as_signup() {
    compose exec -T db mariadb --user=vanillish_signup --password="$signup_password" \
        --batch --skip-column-names realmd -e "$1" 2>&1 || true
}
as_root() {
    # shellcheck disable=SC2016 # the db container expands its own root password
    compose exec -T db sh -c 'exec mariadb --user=root --password="$MARIADB_ROOT_PASSWORD" --batch --skip-column-names "$@"' \
        mariadb "$@"
}

step "compose files"
compose config --quiet
docker compose -f compose.datahost.yaml --env-file "$envfile" config --quiet
echo "  ok  compose.yaml and compose.datahost.yaml"
# Who gets the admin code, read from the resolved files (key names only: the code is never printed).
# CI never starts mangosd, so this is where an environment that lost its database settings shows.
compose config --format json | ADMIN_CODE="$admin" python3 -I -c '
import json, os, sys
code, s = os.environ["ADMIN_CODE"], json.load(sys.stdin)["services"]
env = {n: v.get("environment") or {} for n, v in s.items()}
bad = [f"{n} gets it" for n in ("db", "db-init", "extract", "realmd") if "INILLA_ADMIN_CODE" in env[n]]
bad += [f"{n} does not get it" for n in ("mangosd", "datahost") if env[n].get("INILLA_ADMIN_CODE") != code]
bad += [f"mangosd lost {k}" for k in ("DB_HOST", "DB_PORT", "DB_USER", "DB_PASSWORD", "REALM_TYPE", "TZ")
        if not env["mangosd"].get(k)]
if env["datahost"].get("INILLA_ADMIN_AGENT") != "mangosd:8091":
    bad.append("the data host does not ask mangosd:8091")
if "/vmangos/admin" not in [v.get("target") for v in s["mangosd"].get("volumes", [])]:
    bad.append("mangosd does not mount /vmangos/admin")
if bad:
    sys.exit("FAIL: the admin code: " + "; ".join(bad))
'
echo "  ok  the admin code goes to mangosd (beside its database settings) and the data host, no one else"

step "start: db, db-init, realmd$([ "$datahost" = 1 ] && echo ', datahost') (no game data)"
services=(realmd)
[ "$datahost" = 1 ] && services+=(datahost)
compose up -d "${services[@]}"
# Asked inside the container, as its healthcheck does: Docker's proxy accepts a connection to the
# published port before realmd listens behind it (and closes it then), so a connect proves nothing.
for i in $(seq 1 60); do
    compose exec -T realmd grep -qi ':0E8C 00000000:0000 0A' /proc/net/tcp 2>/dev/null && break
    [ "$i" = 60 ] && fail "realmd is not listening on 3724 in its container"
    sleep 2
done
echo "  ok  realmd listens (published on 127.0.0.1:$realmd_port)"

step "1. the sign-up user"
as_root -e "SHOW GRANTS FOR 'vanillish_signup'@'%'" | sed 's/^/      /'
name="CISQL$(python3 -c 'import secrets; print(secrets.token_hex(4).upper())')"
password="Ci-$(secret | cut -c1-10)"
read -r _ salt verifier < <(python3 ci/srp6.py row "$name" "$password")
out="$(as_signup "SET SESSION sql_mode = 'NO_ENGINE_SUBSTITUTION';
INSERT INTO account (username, v, s) VALUES ('$name', '$verifier', '$salt');
INSERT IGNORE INTO realmcharacters (realmid, acctid, numchars) SELECT id, LAST_INSERT_ID(), 0 FROM realmlist;
SELECT s FROM account WHERE username = '$name';")"
[ "$out" = "$salt" ] || fail "the sign-up user could not add $name the data host's way: $out"
echo "  ok  adds an account and its realmcharacters rows, reads its salt back"
chars="$(as_root realmd -e "SELECT COUNT(*) FROM realmcharacters rc JOIN account a ON a.id = rc.acctid WHERE a.username = '$name'")"
[ "$chars" = 1 ] || fail "expected one realmcharacters row for $name, got $chars"
echo "  ok  the account has its zero character count on the realm"
dup="$(as_signup "INSERT INTO account (username, v, s) VALUES ('$(echo "$name" | tr '[:upper:]' '[:lower:]')', '1', '$salt')")"
case "$dup" in *"ERROR 1062"*) echo "  ok  a name in another case is a duplicate (1062)" ;; *) fail "duplicate name: $dup" ;; esac

denied() {
    local what="$1" sql="$2" got
    got="$(as_signup "$sql")"
    case "$got" in
        *"ERROR 1142"* | *"ERROR 1143"*) echo "  ok  denied: $what" ;;
        *) fail "the sign-up user may $what: $got" ;;
    esac
}
denied "UPDATE account" "UPDATE account SET locked = 1 WHERE username = '$name'"
denied "DELETE FROM account" "DELETE FROM account WHERE username = '$name'"
denied "read v" "SELECT v FROM account WHERE username = '$name'"
denied "insert another column (locked)" "INSERT INTO account (username, v, s, locked) VALUES ('CIX', '1', '$salt', 1)"
denied "insert into account_access (GM)" "INSERT INTO account_access (id, gmlevel, RealmID) VALUES (1, 6, -1)"
denied "change realmlist" "UPDATE realmlist SET address = '10.0.0.1'"
denied "create a table" "CREATE TABLE ci_probe (i INT)"
denied "read the characters database" "SELECT COUNT(*) FROM characters.characters"
denied "read mysql.user" "SELECT COUNT(*) FROM mysql.user"
denied "grant itself more" "GRANT DELETE ON realmd.account TO 'vanillish_signup'@'%'"

again="$(compose run --rm --no-deps db-init 2>&1)" || fail "db-init's second run failed: $again"
case "$again" in
    *"privileges granted"*) fail "db-init's second run reset the sign-up user's privileges" ;;
esac
echo "  ok  db-init again: exit 0, the sign-up user left as it was"
as_root -e "GRANT DELETE ON realmd.account TO 'vanillish_signup'@'%'"
again="$(compose run --rm --no-deps db-init 2>&1)" || fail "db-init after a stray grant failed: $again"
case "$again" in *"privileges granted"*) ;; *) fail "db-init kept a stray grant: $again" ;; esac
denied "DELETE after db-init took the stray grant away" "DELETE FROM account WHERE username = '$name'"

step "2. SRP6 logon with the account the sign-up user made"
out="$(python3 ci/srp6.py logon 127.0.0.1 "$realmd_port" "$name" "$password")" || fail "$out"
echo "      $out"
case "$out" in *": $realm ("* | *", $realm ("*) ;; *) fail "the realm list does not hold $realm" ;; esac
echo "  ok  logon as $name, realm $realm listed"

[ "$datahost" = 1 ] || { printf '\nchecks passed (no data host)\n'; exit 0; }

step "3. the data host: server.json, sign-up, logon (INILLA_SIGNUP=lan)"
python3 ci/accounts_e2e.py open --base "$base" --realmd "127.0.0.1:$realmd_port" --realm "$realm" --world "$world"

step "4. the data host with a token, then with sign-up off"
INILLA_DATA_TOKEN="$token" compose up -d --no-deps datahost
python3 ci/accounts_e2e.py token --base "$base" --token "$token"
INILLA_SIGNUP=off compose up -d --no-deps datahost
python3 ci/accounts_e2e.py closed --base "$base"

step "5. the admin menu's door (ADMIN_CODE set, no mangosd), then INILLA_ADMIN=off"
compose up -d --no-deps datahost
ADMIN_CODE="$admin" python3 -I ci/admin_e2e.py on --base "$base"
INILLA_ADMIN=off compose up -d --no-deps datahost
python3 -I ci/admin_e2e.py off --base "$base"

printf '\nchecks passed\n'
