#!/bin/bash
# The Vanillish server on a Mac with Apple silicon: one command, and safe to run again.
#
#   /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/klanthier/vanillish/klanthier/inilla-android-port/server/setup-mac.sh)"
#
# In order: the server's files, in ~/Vanillish-server; this Mac and its Docker (Docker Desktop,
# OrbStack or Colima); this Mac's address on the network; .env (passwords once, the address on every
# run); your World of Warcraft 1.12.1 archives, copied once into the Docker volume vanillish-wowdata;
# the prebuilt images, started and followed until the realm is up; and what to type on the phone.
# Running it again updates the server and picks up a new address.
#
# Linux and WSL run this same file (setup-linux.sh hands over to it), so the two never drift.
# Written for the bash 3.2 that macOS ships and its BSD tools: no arrays, no ${x,,}, no mapfile, no
# &>>, no sed -i, stat -c, readlink -f or timeout. It needs neither git nor python: on a fresh Mac
# both are stubs that ask to install the developer tools.
set -eu
set -o pipefail

REPO="${VANILLISH_REPO:-klanthier/vanillish}"
REF="${VANILLISH_REF:-klanthier/inilla-android-port}"
# The player's archives live in this volume; compose.yaml mounts it read-only at /client/Data.
VOLUME="${VANILLISH_DATA_VOLUME:-vanillish-wowdata}"
# A throwaway container that writes into (and reads) that volume.
HELPER="${VANILLISH_HELPER_IMAGE:-alpine:3}"
PROJECT="${COMPOSE_PROJECT_NAME:-vanillish}"
# The archives a 1.12.1 client cannot start without, in their stock spelling: the extractors and
# the data host open them by these names (datahost/src/manifest.rs, REQUIRED).
REQUIRED="dbc.MPQ fonts.MPQ interface.MPQ misc.MPQ model.MPQ patch.MPQ sound.MPQ speech.MPQ terrain.MPQ texture.MPQ wmo.MPQ"
NL='
'

TTY=0
B="" D="" Y="" R="" G="" N=""
if [ -t 1 ]; then
    TTY=1
    if [ -z "${NO_COLOR:-}" ]; then
        B=$'\033[1m' D=$'\033[2m' Y=$'\033[33m' R=$'\033[31m' G=$'\033[32m' N=$'\033[0m'
    fi
fi
say() { printf '\n%s==>%s %s%s%s\n' "$G" "$N" "$B" "$*" "$N"; }
info() { printf '    %s\n' "$*"; }
note() { printf '%s    %s%s\n' "$D" "$*" "$N" >&2; }
warn() { printf '%s !  %s%s\n' "$Y" "$*" "$N" >&2; }
die() {
    end_line
    printf '%s xx %s%s\n' "$R" "$*" "$N" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Sets up the Vanillish server (a vmangos 1.12.1 realm and the game-data host) in Docker, and
prints the address to type in the Vanillish app.

  setup-mac.sh [options] [WOW_FOLDER]

  WOW_FOLDER, --data DIR  your World of Warcraft 1.12.1 folder, or its Data folder (also
                          VANILLISH_WOW_DIR). Without it: a folder dialog, once.
  --reimport              copy the game files again, even when the volume already holds them
  --ip ADDRESS            the address the phone reaches this machine at (default: found on the LAN;
                          also VANILLISH_IP)
  --mmaps, --no-mmaps     creature pathfinding: generated in the background, hours (asked once)
  -y, --yes               take the default answer to every question
  --no-update             keep the server files that are here; download nothing
  -n, --dry-run           change nothing: show what would be done (on Linux this also rehearses the
                          macOS path with VANILLISH_PLATFORM=mac, skipping the macOS-only calls)
  --uninstall             stop the server and remove it (asks before deleting anything)
  -h, --help              this text

The files go to ~/Vanillish-server (VANILLISH_DIR). Run from a checkout's server/ folder, it uses
that folder instead.
EOF
}

# ---- small helpers ---------------------------------------------------------------------------

lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }
gb() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1000000000 }'; }
mmss() { printf '%dm%02ds' $(($1 / 60)) $(($1 % 60)); }
is_private_v4() {
    case "$1" in
        10.* | 192.168.* | 172.1[6-9].* | 172.2[0-9].* | 172.3[01].*) return 0 ;;
        *) return 1 ;;
    esac
}

# Questions read the terminal itself, so they work under `bash -c "$(curl …)"` and in a pipe.
have_tty() { [ "$ASSUME_YES" = 0 ] && (: </dev/tty) 2>/dev/null; }
ask_yn() { # ask_yn QUESTION y|n — the answer (the default without a terminal, or with --yes)
    local a=""
    if ! have_tty; then
        [ "$2" = y ]
        return
    fi
    if [ "$2" = y ]; then printf '%s?%s %s [Y/n] ' "$B" "$N" "$1" >/dev/tty; else printf '%s?%s %s [y/N] ' "$B" "$N" "$1" >/dev/tty; fi
    IFS= read -r a </dev/tty || a=""
    case "$a" in
        [Yy]*) return 0 ;;
        [Nn]*) return 1 ;;
        *) [ "$2" = y ] ;;
    esac
}
ask_line() { # ask_line PROMPT — one typed line ("" without a terminal)
    local a=""
    if have_tty; then
        printf '%s?%s %s' "$B" "$N" "$1" >/dev/tty
        IFS= read -r a </dev/tty || a=""
    fi
    printf '%s' "$a"
}

# A macOS-only call. Elsewhere (a --dry-run rehearsal of the macOS path on Linux) it is skipped,
# and fails, so every caller has its fallback.
mac() {
    if [ "$ON_MAC" = 1 ]; then
        "$@"
        return
    fi
    note "(dry run: skipped the macOS-only call: $*)"
    return 1
}
# act CMD… — something that changes this machine; under --dry-run only shown.
act() {
    if [ "$DRY" = 1 ]; then
        note "(dry run: would run: $*)"
        return 0
    fi
    "$@"
}
docker_ok() { [ "$DOCKER_OK" = 1 ]; }

# The folder as the summary shows it: ~/… when it is under the home folder, quoted with spaces.
disp_dir() {
    local d="$DIR"
    # shellcheck disable=SC2088 # a literal ~, for the reader
    case "$d" in "$HOME"/*) d="~/${d#"$HOME"/}" ;; esac
    case "$d" in *' '*) d="\"$DIR\"" ;; esac
    printf '%s' "$d"
}

# ---- arguments, platform, where the server lives -----------------------------------------------

parse_args() {
    DRY=0 ASSUME_YES=0 REIMPORT=0 NO_UPDATE=0 UNINSTALL=0
    MMAPS="" WOW_SRC="${VANILLISH_WOW_DIR:-}" IP_ARG="${VANILLISH_IP:-}"
    while [ $# -gt 0 ]; do
        case "$1" in
            --data | --ip)
                [ $# -ge 2 ] || die "$1 needs a value"
                if [ "$1" = --data ]; then WOW_SRC="$2"; else IP_ARG="$2"; fi
                shift
                ;;
            --data=*) WOW_SRC="${1#*=}" ;;
            --ip=*) IP_ARG="${1#*=}" ;;
            --mmaps) MMAPS=1 ;;
            --no-mmaps) MMAPS=0 ;;
            --reimport) REIMPORT=1 ;;
            -y | --yes) ASSUME_YES=1 ;;
            --no-update) NO_UPDATE=1 ;;
            -n | --dry-run) DRY=1 ;;
            --uninstall) UNINSTALL=1 ;;
            -h | --help)
                usage
                exit 0
                ;;
            -*) die "unknown option: $1 (see --help)" ;;
            *) WOW_SRC="$1" ;;
        esac
        shift
    done
}

detect_platform() {
    ON_MAC=0
    if [ "$(uname -s)" = Darwin ]; then ON_MAC=1; fi
    OS="${VANILLISH_PLATFORM:-}"
    if [ -z "$OS" ]; then
        case "$(uname -s)" in
            Darwin) OS=mac ;;
            Linux) OS=linux ;;
            *) die "this setup runs on macOS and Linux (on Windows: inside WSL)" ;;
        esac
    fi
    case "$OS" in mac | linux) ;; *) die "VANILLISH_PLATFORM is mac or linux" ;; esac
    if [ "$OS" = mac ] && [ "$ON_MAC" = 0 ] && [ "$DRY" = 0 ]; then
        die "the macOS path runs on another system only as a rehearsal: add --dry-run"
    fi
    WSL=0
    if [ "$OS" = linux ]; then
        case "$(uname -r)" in *[Mm]icrosoft* | *WSL*) WSL=1 ;; esac
    fi
}

# fresh: nothing here yet; installed: a folder this setup made (updated on every run);
# checkout: the server/ folder of a clone, used as it is.
locate_dir() {
    local here=""
    SELF_FILE=""
    case "$0" in
        *setup-mac.sh)
            if [ -f "$0" ]; then
                here="$(cd "$(dirname "$0")" && pwd)"
                SELF_FILE="$here/setup-mac.sh"
            fi
            ;;
    esac
    if [ -n "$here" ] && [ -f "$here/compose.yaml" ]; then DIR="$here"; else DIR="${VANILLISH_DIR:-$HOME/Vanillish-server}"; fi
    MODE=fresh
    if [ -f "$DIR/.vanillish-setup" ]; then
        MODE=installed
        if [ -z "${VANILLISH_REPO:-}" ]; then REPO="$(sed -n 's/^repo=//p' "$DIR/.vanillish-setup")"; fi
        if [ -z "${VANILLISH_REF:-}" ]; then REF="$(sed -n 's/^ref=//p' "$DIR/.vanillish-setup")"; fi
        [ -n "$REPO" ] || REPO=klanthier/vanillish
        [ -n "$REF" ] || REF=klanthier/inilla-android-port
    elif [ -f "$DIR/compose.yaml" ] && [ "$DIR" = "$here" ]; then
        MODE=checkout
    elif [ -f "$DIR/compose.yaml" ]; then
        MODE=installed
    fi
}

# ---- 1. this machine ---------------------------------------------------------------------------

check_machine() {
    local chip ver
    MAC_ARM=0
    if [ "$OS" = linux ]; then
        say "This machine: $(uname -sm)$([ "$WSL" = 1 ] && printf ' (WSL)')"
        return 0
    fi
    chip="$(mac sysctl -n machdep.cpu.brand_string || echo 'an unknown chip')"
    ver="$(mac sw_vers -productVersion || echo '?')"
    say "This Mac: $chip, macOS $ver"
    # 1 on Apple silicon, even from a Terminal under Rosetta; an Intel Mac has no such key. A
    # rehearsal on another system takes Apple silicon.
    MAC_ARM="$(mac sysctl -n hw.optional.arm64 2>/dev/null || true)"
    if [ -z "$MAC_ARM" ] && [ "$ON_MAC" = 0 ]; then MAC_ARM=1; fi
    if [ "$MAC_ARM" != 1 ]; then
        warn "an Intel Mac: the server runs here too, but this setup is made for Apple silicon"
    elif [ "$(mac sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
        note "this Terminal runs under Rosetta; that is fine, Docker still runs natively"
    fi
}

# ---- 2. Docker ---------------------------------------------------------------------------------

# Which runtime this Mac has: orbstack, desktop, colima, or nothing.
mac_runtime() {
    case "$(docker context show 2>/dev/null || true)" in
        orbstack) echo orbstack ;;
        desktop-linux) echo desktop ;;
        colima*) echo colima ;;
        *)
            if [ -d /Applications/OrbStack.app ]; then
                echo orbstack
            elif [ -d /Applications/Docker.app ]; then
                echo desktop
            elif command -v colima >/dev/null 2>&1; then
                echo colima
            fi
            ;;
    esac
}
runtime_label() {
    case "$1" in
        orbstack) echo OrbStack ;;
        desktop) echo "Docker Desktop" ;;
        colima) echo Colima ;;
        *) echo Docker ;;
    esac
}

no_docker() {
    if [ "$DRY" = 1 ]; then
        warn "Docker is not installed or not running here; the dry run goes on without it"
        DOCKER_OK=0
        return 0
    fi
    if [ "$OS" = mac ]; then
        if ask_yn "Docker is not installed. Open the OrbStack download page?" y; then
            mac open "https://orbstack.dev/download" || true
        fi
        die "Install one of these, open it once, then run this command again:
      OrbStack (light; free for personal use)  https://orbstack.dev/download    or: brew install --cask orbstack
      Docker Desktop                           https://www.docker.com/products/docker-desktop/    or: brew install --cask docker-desktop"
    fi
    die "Docker is not installed. Install Docker Engine and its compose plugin (https://docs.docker.com/engine/install/),
      or on Windows Docker Desktop with WSL integration for this distribution, then run this again."
}

wait_docker() { # wait_docker SECONDS
    local i=0
    while ! docker info >/dev/null 2>&1; do
        i=$((i + 2))
        [ "$i" -lt "$1" ] || return 1
        sleep 2
    done
}

start_docker() {
    local rt label err
    if [ "$DRY" = 1 ]; then
        no_docker
        return 0
    fi
    if [ "$OS" = mac ]; then
        rt="$(mac_runtime)"
        [ -n "$rt" ] || no_docker
        label="$(runtime_label "$rt")"
        ask_yn "$label is not running. Start it now?" y || die "Start $label, then run this again."
        info "starting $label (the first start can ask you to accept its terms)…"
        case "$rt" in
            orbstack) open -a OrbStack ;;
            desktop) open -a Docker ;;
            colima) colima start --cpu 4 --memory 6 ;;
        esac
        wait_docker 180 || die "$label did not start within 3 minutes: open it, then run this again"
        return 0
    fi
    err="$(docker info 2>&1 >/dev/null || true)"
    case "$err" in
        *"permission denied"*)
            die "Docker runs, but this user may not use it. Run: sudo usermod -aG docker \"\$USER\"
      then log out and back in, and run this again."
            ;;
    esac
    if [ "$WSL" = 1 ]; then
        die "Docker is not running. Start Docker Desktop on Windows, turn on Settings > Resources >
      WSL integration for this distribution, then run this again."
    fi
    if command -v systemctl >/dev/null 2>&1 && ask_yn "Docker is not running. Start it (sudo systemctl start docker)?" y; then
        sudo systemctl start docker
        wait_docker 60 || die "Docker did not start: sudo systemctl status docker"
        return 0
    fi
    die "Docker is not running. Start it, then run this again."
}

check_docker() {
    local mem cpus
    say "Docker"
    DOCKER_OK=1 RUNTIME=""
    if ! command -v docker >/dev/null 2>&1; then
        no_docker
        return 0
    fi
    if ! docker info >/dev/null 2>&1; then
        start_docker
        docker_ok || return 0
    fi
    docker compose version >/dev/null 2>&1 ||
        die "'docker compose' is missing (Colima: brew install docker-compose; Linux: the docker-compose-plugin package)"
    if [ "$OS" = mac ]; then
        # The engine names itself; the context and the apps installed are only a guess (both apps
        # can be there, with the default context).
        case "$(docker info --format '{{.OperatingSystem}}' 2>/dev/null || true)" in
            *OrbStack*) RUNTIME=orbstack ;;
            *"Docker Desktop"*) RUNTIME=desktop ;;
            *) RUNTIME="$(mac_runtime)" ;;
        esac
    fi
    DOCKER_ARCH="$(docker info --format '{{.Architecture}}' 2>/dev/null || echo '?')"
    mem="$(docker info --format '{{.MemTotal}}' 2>/dev/null || echo 0)"
    cpus="$(docker info --format '{{.NCPU}}' 2>/dev/null || echo '?')"
    case "$mem" in '' | *[!0-9]*) mem=0 ;; esac
    info "$(runtime_label "$RUNTIME") $(docker version --format '{{.Server.Version}}' 2>/dev/null || true): $cpus CPUs, $(gb "$mem") GB of memory, $DOCKER_ARCH"
    if [ "$MAC_ARM" = 1 ]; then
        case "$DOCKER_ARCH" in
            aarch64 | arm64) ;;
            *) warn "Docker's engine is $DOCKER_ARCH, not arm64: the server would run emulated, and slowly" ;;
        esac
    fi
    if [ "$mem" -gt 0 ] && [ "$mem" -lt 3800000000 ]; then
        warn "Docker has $(gb "$mem") GB of memory; the realm wants 4 GB or more."
        case "$RUNTIME" in
            desktop) warn "  Docker Desktop > Settings > Resources > Memory limit" ;;
            orbstack) warn "  OrbStack > Settings > System > Memory limit" ;;
            colima) warn "  colima stop && colima start --cpu 4 --memory 6" ;;
        esac
    fi
}

# ---- 3. the server's files ---------------------------------------------------------------------

# Downloads the repository's tarball (about 10 MB) and takes server/ out of it. What the player
# may have edited survives: .env, config/*.conf, and the addons they added.
fetch_stack() {
    local url="https://codeload.github.com/$REPO/tar.gz/$REF" tgz="$TMP/src.tgz" top stage keep
    say "Server files: $REPO @ $REF"
    curl -fsSL --retry 3 --connect-timeout 20 -o "$tgz" "$url" ||
        die "could not download $url (is this machine online?)"
    top="$(tar -tzf "$tgz" | sed -n 's|^\([^/]*\)/server/compose\.yaml$|\1|p')"
    top="${top%%"$NL"*}"
    [ -n "$top" ] || die "the download has no server/ folder: $url"
    tar -xzf "$tgz" -C "$TMP" "$top/server" || die "could not unpack $url"
    stage="$TMP/$top/server"
    for keep in config/mangosd.conf config/realmd.conf; do
        if [ -f "$DIR/$keep" ]; then rm -f "$stage/$keep"; fi
    done
    if [ -d "$DIR/addons" ]; then
        cp "$stage/addons/README.md" "$TMP/addons-README.md"
        rm -rf "$stage/addons"
        mkdir "$stage/addons"
        mv "$TMP/addons-README.md" "$stage/addons/README.md"
    fi
    rm -f "$stage/.env"
    if [ "$DRY" = 1 ]; then
        note "(dry run: would copy $(find "$stage" -type f | wc -l | tr -d ' ') files into $DIR)"
        STAGE="$stage"
        return 0
    fi
    mkdir -p "$DIR"
    # A copy of this script that is running keeps reading its old file: remove, never overwrite.
    rm -f "$DIR/setup-mac.sh" "$DIR/setup-linux.sh"
    cp -R "$stage/." "$DIR/"
    printf 'repo=%s\nref=%s\n' "$REPO" "$REF" >"$DIR/.vanillish-setup"
    info "in $(disp_dir)"
}

# ---- 4. this machine's address -----------------------------------------------------------------

# The default route's interface first (Wi-Fi or Ethernet, whichever macOS prefers), then any en*
# with a private IPv4. A VPN (utun*) default route is skipped: the phone is on the LAN, not the
# VPN. Never assume en0: on a Mac mini it is Ethernet, on a MacBook Wi-Fi.
mac_lan_ip() {
    local ifc ip=""
    ifc="$(mac route -n get default 2>/dev/null | awk '/interface:/ { print $2 }' || true)"
    case "$ifc" in en*) ip="$(mac ipconfig getifaddr "$ifc" 2>/dev/null || true)" ;; esac
    if [ -z "$ip" ]; then
        for ifc in $(mac ifconfig -l 2>/dev/null || true); do
            case "$ifc" in en*) ;; *) continue ;; esac
            ip="$(mac ipconfig getifaddr "$ifc" 2>/dev/null || true)"
            if is_private_v4 "$ip"; then break; fi
            ip=""
        done
    fi
    printf '%s' "$ip"
}

# On WSL the phone reaches Windows, which publishes Docker Desktop's ports: Windows' own address.
wsl_windows_ip() {
    command -v powershell.exe >/dev/null 2>&1 || return 0
    # shellcheck disable=SC2016 # $_ is PowerShell's
    powershell.exe -NoProfile -NonInteractive -Command '(Get-NetIPConfiguration | Where-Object { $_.IPv4DefaultGateway -ne $null -and $_.NetAdapter.Status -eq "Up" } | Select-Object -First 1).IPv4Address.IPAddress' 2>/dev/null |
        tr -d '\r' | sed -n 1p || true
}

linux_lan_ip() {
    local ip="" a
    if [ "$WSL" = 1 ]; then ip="$(wsl_windows_ip)"; fi
    if [ -z "$ip" ] && command -v ip >/dev/null 2>&1; then
        ip="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{ for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1) }' | sed -n 1p || true)"
    fi
    if [ -z "$ip" ]; then
        # Docker's own bridges use 172.17-31 first, so a home network's 192.168 or 10 wins.
        for a in $(hostname -I 2>/dev/null || true); do
            case "$a" in 192.168.* | 10.*) ip="$a" && break ;; esac
        done
    fi
    printf '%s' "$ip"
}

find_ip() {
    say "This machine's address on your network"
    LOCALNAME=""
    if [ -n "$IP_ARG" ]; then
        IP="$IP_ARG"
    elif [ "$OS" = mac ]; then
        IP="$(mac_lan_ip)"
    else
        IP="$(linux_lan_ip)"
    fi
    if [ -z "$IP" ] && [ "$DRY" = 1 ]; then
        IP=192.0.2.10
        note "(dry run: no LAN address found; going on with $IP)"
    fi
    if [ -z "$IP" ]; then
        IP="$(ask_line "No LAN address found. Type the address the phone reaches this machine at: ")"
    fi
    [ -n "$IP" ] || die "no LAN address found: is this machine on Wi-Fi or Ethernet? (or pass --ip ADDRESS)"
    case "$IP" in *[!A-Za-z0-9.-]*) die "'$IP' is not an IPv4 address or a host name" ;; esac
    [ "${#IP}" -le 32 ] || die "'$IP' is longer than the 32 characters a realm address holds"
    if [ "$OS" = mac ]; then LOCALNAME="$(mac scutil --get LocalHostName 2>/dev/null || true)"; fi
    info "$IP${LOCALNAME:+   (and $LOCALNAME.local)}"
    if [ "$WSL" = 1 ] && [ -z "$IP_ARG" ]; then
        note "WSL: the phone reaches Windows, where Docker Desktop publishes the server's ports. If this"
        note "is not Windows' address (ipconfig, in Windows), run this again with --ip ADDRESS."
    fi
}

# ---- 5. .env -----------------------------------------------------------------------------------

env_get() { # env_get KEY — its value in .env ("" when absent), surrounding quotes removed
    [ -f "$ENV_FILE" ] || return 0
    awk -v k="$1" 'index($0, k "=") == 1 { v = substr($0, length(k) + 2) }
        END { sub(/\r$/, "", v); if (v ~ /^".*"$/ || v ~ /^'\''.*'\''$/) v = substr(v, 2, length(v) - 2); print v }' "$ENV_FILE"
}
env_set() { # env_set KEY VALUE — rewrite KEY's line or append it (no sed -i: BSD and GNU differ)
    awk -v k="$1" -v v="$2" 'BEGIN { done = 0 }
        index($0, k "=") == 1 { if (!done) print k "=" v; done = 1; next }
        { print }
        END { if (!done) print k "=" v }' "$ENV_FILE" >"$ENV_FILE.tmp"
    cat "$ENV_FILE.tmp" >"$ENV_FILE" # in place, so .env keeps its 0600
    rm -f "$ENV_FILE.tmp"
}
env_comment() { # env_comment KEY — turn KEY's line into a comment
    awk -v k="$1" 'index($0, k "=") == 1 { print "# " $0; next } { print }' "$ENV_FILE" >"$ENV_FILE.tmp"
    cat "$ENV_FILE.tmp" >"$ENV_FILE"
    rm -f "$ENV_FILE.tmp"
}
secret() {
    local s=""
    s="$(openssl rand -hex 16 2>/dev/null || true)"
    if [ "${#s}" -ne 32 ]; then s="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"; fi
    [ "${#s}" -eq 32 ] || die "could not make a random password"
    printf '%s' "$s"
}
host_tz() { # this machine's time zone (the realm's clock: day and night, resets), else UTC
    local z
    z="$(readlink /etc/localtime 2>/dev/null || true)"
    z="${z##*zoneinfo/}"
    case "$z" in '' | /* | *[!A-Za-z0-9_+/-]*) z=UTC ;; esac
    printf '%s' "$z"
}

# The database of an earlier install is here, but not the .env it was made with (its folder was
# deleted): MariaDB keeps the root password it was created with, so new passwords cannot open it,
# and db-init would fail on every start. Deletes it when the player agrees (0), else stops.
orphan_database() {
    warn "this machine has the database of an earlier Vanillish server (the Docker volume ${PROJECT}_db),"
    warn "but not the .env with its passwords: new passwords cannot open it."
    if [ "$DRY" = 1 ]; then
        note "(dry run: a real run asks whether to delete that old database)"
        return 1
    fi
    if ask_yn "Delete that old database (its accounts and characters) and start afresh" n; then
        # Its containers go first (they hold the volume); the start below makes them again.
        docker ps -aq --filter "label=com.docker.compose.project=$PROJECT" | while read -r c; do
            docker rm -f "$c" >/dev/null || true
        done
        docker volume rm "${PROJECT}_db" >/dev/null || die "could not delete the volume ${PROJECT}_db"
        info "deleted the old database"
        return 0
    fi
    die "nothing was changed. Put the earlier .env back in $(disp_dir), or delete the old database:
      run this again and answer yes, or: docker volume rm ${PROJECT}_db (with its containers removed)"
}

write_env() {
    local key cur old db=0 p1 p2 p3
    say "Settings (.env)"
    if docker_ok && docker volume inspect "${PROJECT}_db" >/dev/null 2>&1; then db=1; fi
    if [ ! -f "$ENV_FILE" ] || [ -z "$(env_get DB_ROOT_PASSWORD)" ]; then
        if [ "$db" = 1 ] && orphan_database; then db=0; fi
    fi
    if [ ! -f "$ENV_FILE" ]; then
        p1="$(secret)"
        p2="$(secret)"
        p3="$(secret)"
        (umask 077 && : >"$ENV_FILE")
        cat >>"$ENV_FILE" <<EOF
# The Vanillish server's settings, written by its setup; .env.example explains every one.
# After an edit, run the setup again (or: docker compose up -d).

# Generated for this machine. The database is created with them: keep them.
DB_ROOT_PASSWORD=$p1
DB_PASSWORD=$p2
SIGNUP_DB_PASSWORD=$p3

# The address the phone reaches this machine at; the setup refreshes it on every run.
REALM_ADDRESS=$IP
REALM_NAME=Vanillish
REALM_TYPE=pve
TZ=$(host_tz)

# Accounts made from the app: lan (phones on this network; the default), off, or open.
INILLA_SIGNUP=lan
EOF
        info "new database passwords, in $(disp_dir)/.env"
    else
        for key in DB_ROOT_PASSWORD DB_PASSWORD SIGNUP_DB_PASSWORD; do
            cur="$(env_get "$key")"
            case "$cur" in
                '')
                    env_set "$key" "$(secret)"
                    info "added a new $key"
                    ;;
                change-me*)
                    # MariaDB keeps the root password it was first created with; db-init gives the
                    # other two users the .env's passwords on every start.
                    if [ "$db" = 1 ] && [ "$key" = DB_ROOT_PASSWORD ]; then
                        warn "$key is still the example from .env.example; the database was made with it, so it stays"
                    else
                        env_set "$key" "$(secret)"
                        info "replaced the example $key with a new one"
                    fi
                    ;;
            esac
        done
        if [ -z "$(env_get REALM_NAME)" ]; then env_set REALM_NAME Vanillish; fi
        if [ -z "$(env_get TZ)" ]; then env_set TZ "$(host_tz)"; fi
        old="$(env_get REALM_ADDRESS)"
        if [ "$old" != "$IP" ]; then
            env_set REALM_ADDRESS "$IP"
            if [ -n "$old" ]; then info "the address changed: $old -> $IP"; fi
        fi
        info "kept the passwords in $(disp_dir)/.env"
    fi
    chmod 600 "$ENV_FILE"
    # The data host's port, as compose reads it: the shell's first, then .env's.
    PORT="${DATAHOST_PORT:-$(env_get DATAHOST_PORT)}"
    case "$PORT" in '' | *[!0-9]*) PORT=8099 ;; esac
}

# typed HOST — what the player types in the app: HOST, with the data host's port when it is not the
# app's default.
typed() { if [ "$PORT" = 8099 ]; then printf '%s' "$1"; else printf '%s:%s' "$1" "$PORT"; fi; }

# ---- 6. the game files -------------------------------------------------------------------------

# entry DIR NAME — the name of DIR's entry that is NAME in any case (a Mac keeps DBC.MPQ as it came;
# Linux is case-sensitive). No `ls | grep`: with pipefail an early exit fails the pipe.
entry() {
    local p f want
    want="$(lower "$2")"
    for p in "$1"/*; do
        f="${p##*/}"
        [ "${#f}" -eq "${#want}" ] || continue
        if [ -e "$p" ] && [ "$(lower "$f")" = "$want" ]; then
            printf '%s' "$f"
            return 0
        fi
    done
    return 1
}
# find_data DIR — the 1.12 Data folder: DIR itself, DIR/Data, DIR/<a folder>, or DIR/<a folder>/Data.
find_data() {
    local d sub
    for d in "$1" "$1"/*; do
        [ -d "$d" ] || continue
        if entry "$d" dbc.MPQ >/dev/null; then
            printf '%s' "$d"
            return 0
        fi
        if sub="$(entry "$d" Data)" && [ -d "$d/$sub" ] && entry "$d/$sub" dbc.MPQ >/dev/null; then
            printf '%s' "$d/$sub"
            return 0
        fi
    done
    return 1
}

# One 1.12 install in the usual places that need no permission to look into, or nothing. Desktop,
# Documents and Downloads are left alone: macOS asks before Terminal reads them.
guess_wow_folder() {
    local base d hit="" n=0
    for base in /Applications "$HOME/Applications" "$HOME/Games" "$HOME"; do
        for d in "$base"/*; do
            [ -d "$d" ] || continue
            case "${d##*/}" in Applications | Desktop | Documents | Downloads | Games | Library | Movies | Music | Pictures | Public) continue ;; esac
            if [ -d "$d/Data" ] && entry "$d/Data" dbc.MPQ >/dev/null; then
                hit="$d"
                n=$((n + 1))
            fi
        done
    done
    if [ "$n" = 1 ]; then printf '%s' "$hit"; fi
}

mac_pick_folder() {
    mac osascript 2>/dev/null <<'OSA' || true
try
    activate
    POSIX path of (choose folder with prompt "Choose your World of Warcraft 1.12.1 folder (or its Data folder):")
on error
    ""
end try
OSA
}

# A typed or dragged path, made plain: Terminal escapes a dragged one ("World\ of\ Warcraft"), some
# shells quote it, WSL gets C:\…, and ~ is the home folder.
clean_path() {
    local p="$1" quoted=0
    p="$(printf '%s' "$p" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
    case "$p" in
        \'*\' | \"*\")
            p="${p#?}"
            p="${p%?}"
            quoted=1
            ;;
    esac
    case "$p" in
        [A-Za-z]:\\* | [A-Za-z]:/*)
            if command -v wslpath >/dev/null 2>&1; then p="$(wslpath -u "$p")"; fi
            ;;
        *)
            if [ "$quoted" = 0 ]; then p="$(printf '%s' "$p" | sed -e 's/\\\(.\)/\1/g')"; fi
            ;;
    esac
    # shellcheck disable=SC2088 # a typed ~, matched literally
    case "$p" in
        "~") p="$HOME" ;;
        "~/"*) p="$HOME/${p#"~/"}" ;;
    esac
    printf '%s' "$p"
}

# The folder the player picks: a guess they confirm, the Finder dialog, or a typed or dragged path.
choose_wow_folder() {
    local d
    if [ "$OS" = mac ]; then
        d="$(guess_wow_folder)"
        if [ -n "$d" ] && ask_yn "Use the World of Warcraft folder in $d" y; then
            printf '%s' "$d"
            return 0
        fi
        if have_tty; then
            info "Choose your World of Warcraft 1.12.1 folder in the window that opens (it can open behind this one)" >&2
            d="$(mac_pick_folder)"
            if [ -n "$d" ]; then
                printf '%s' "$d"
                return 0
            fi
        fi
    fi
    ask_line "Drag your World of Warcraft 1.12.1 folder into this window (or type its path), then press Return:
  > "
}

# client_locale DATA — the language WTF/Config.wtf beside DATA names ("" when it names none).
client_locale() {
    local root wtf cfg
    root="$(dirname "$1")"
    wtf="$(entry "$root" WTF)" || return 0
    cfg="$(entry "$root/$wtf" Config.wtf)" || return 0
    sed -n 's/^[Ss][Ee][Tt][[:space:]][[:space:]]*[Ll]ocale[[:space:]][[:space:]]*"\([A-Za-z][A-Za-z][A-Za-z][A-Za-z]\)".*/\1/p' "$root/$wtf/$cfg" | sed -n 1p
}

check_language() {
    local loc
    loc="$(client_locale "$1")"
    case "$loc" in
        '') info "language: English (enUS) assumed; Vanillish is made for an English 1.12.1 client" ;;
        enUS | enGB) info "language: English ($loc)" ;;
        *)
            warn "this client is $loc, not English (enUS): Vanillish is made and tested with English 1.12.1."
            warn "Another language mostly works, untested (set WOW_LOCALE=$loc in .env so the app is told)."
            ask_yn "Use it anyway" y || die "nothing was copied"
            ;;
    esac
}

# plan_import DATA — sets PLAN ("<name as found> <stock name> <bytes>" per line), TOTAL and COUNT:
# the 11 required archives, then patch-?.MPQ and speech2.MPQ. Repack extras (base.MPQ, backup.MPQ)
# and the Interface, WTF and Cache folders stay behind: the client never opens them.
plan_import() {
    local data="$1" want f p canon missing="" seen=" " n
    PLAN="" TOTAL=0 COUNT=0
    for want in $REQUIRED; do
        if f="$(entry "$data" "$want")" && [ -f "$data/$f" ]; then
            PLAN="$PLAN$f $want$NL"
            seen="$seen$want "
        else
            missing="$missing $want"
        fi
    done
    [ -z "$missing" ] || die "$data is not a complete 1.12.1 Data folder; it is missing:$missing"
    for p in "$data"/*; do
        f="${p##*/}"
        [ -f "$p" ] || continue
        case "$(lower "$f")" in
            patch-?.mpq | speech2.mpq)
                canon="$(lower "${f%.*}").MPQ"
                case "$seen" in *" $canon "*) continue ;; esac
                PLAN="$PLAN$f $canon$NL"
                seen="$seen$canon "
                ;;
        esac
    done
    # Sizes last, one read per archive. A refusal here is macOS privacy (TCC), most likely.
    p="$PLAN"
    PLAN=""
    while read -r f canon; do
        [ -n "$f" ] || continue
        n="$(wc -c <"$data/$f" 2>/dev/null)" || die_read "$data"
        n=$((n))
        PLAN="$PLAN$f $canon $n$NL"
        TOTAL=$((TOTAL + n))
        COUNT=$((COUNT + 1))
    done <<EOF
$p
EOF
}
plan_list() { printf '%s' "$PLAN" | awk 'NF { print $2, $3 }' | LC_ALL=C sort; }
die_read() {
    die "could not copy the game files from $1 (the messages above say why).
      If macOS asked whether Terminal may open that folder, allow it (System Settings > Privacy &
      Security > Files and Folders, or Full Disk Access), then run this again."
}

# "<name> <bytes>" for each archive in the volume, sorted; nothing when there is no volume.
volume_list() {
    docker volume inspect "$VOLUME" >/dev/null 2>&1 || return 0
    if [ "$DRY" = 1 ] && ! docker image inspect "$HELPER" >/dev/null 2>&1; then return 0; fi
    docker run --rm -v "$VOLUME":/d:ro "$HELPER" \
        sh -c 'cd /d && for f in *.MPQ; do [ -f "$f" ] && echo "$f $(stat -c %s "$f")"; done; true' |
        LC_ALL=C sort
}
has_required() {
    local w
    for w in $REQUIRED; do
        case "$NL$1" in *"$NL$w "*) ;; *) return 1 ;; esac
    done
}
tar_knows() { tar -c "$1" -f - -C "$TMP/empty" . >/dev/null 2>&1; }

# Inside the throwaway container: the new archives land in .incoming, and only once every one of
# them is there at its full size do the old ones go and the new ones take their stock names (tar
# skips a file it cannot read and still ends the archive cleanly, so a whole stream is not proof).
# A copy that stops half-way leaves the old archives as they were, and nothing behind. 0444:
# nothing writes them. RENAMES: "<name as sent> <stock name> <bytes>" per line.
# shellcheck disable=SC2016 # the container's shell expands these
IMPORT_SH='
trap "rm -rf /d/.incoming" EXIT
trap "exit 1" HUP INT TERM
rm -rf /d/.incoming
mkdir /d/.incoming
tar -xf - -C /d/.incoming
echo "$RENAMES" | while read -r found canon size; do
    [ -n "$found" ] || continue
    got="$(stat -c %s "/d/.incoming/$found" 2>/dev/null || echo 0)"
    if [ "$got" != "$size" ]; then
        echo "$found arrived with $got of its $size bytes; the archives already there are kept" >&2
        exit 1
    fi
done
for f in /d/*; do
    case "$(basename "$f" | tr A-Z a-z)" in *.mpq) rm -f "$f" ;; esac
done
echo "$RENAMES" | while read -r found canon size; do
    [ -n "$found" ] || continue
    mv "/d/.incoming/$found" "/d/$canon"
done
chown 0:0 /d/*.MPQ
chmod 0444 /d/*.MPQ
cd /d
for f in *.MPQ; do echo "$f $(stat -c %s "$f")"; done
'

# Streams the archives as one tar through the Docker API into the volume: no file sharing, so it
# works the same on Docker Desktop, OrbStack and Colima, from any disk, and the realm keeps
# running when the WoW folder moves, its drive is unplugged or macOS withdraws the permission.
import_archives() {
    local data="$1" names renames want got flags="" t0 secs avail
    names="$(printf '%s' "$PLAN" | awk 'NF { print $1 }')"
    renames="$(printf '%s' "$PLAN" | awk 'NF { print $1, $2, $3 }')"
    want="$(plan_list)"
    # macOS's bsdtar would add AppleDouble ._ files and extended attributes (com.apple.quarantine).
    if tar_knows --no-mac-metadata; then flags="--no-mac-metadata "; fi
    if tar_knows --no-xattrs; then flags="$flags--no-xattrs"; fi
    say "Copying $COUNT archives ($(gb "$TOTAL") GB) into the Docker volume $VOLUME"
    info "a minute or so; nothing leaves this machine"
    if [ "$DRY" = 1 ]; then
        # shellcheck disable=SC2086 # one line, names joined
        note "(dry run: would stream: COPYFILE_DISABLE=1 tar -c $flags -f - -C \"$data\" $(printf '%s ' $names)| docker run --rm -i -v $VOLUME:/d $HELPER sh -c …)"
        return 0
    fi
    # The data host reads these files: it starts again after the copy (and indexes them again).
    if [ -n "$(docker compose ps -q --status running datahost 2>/dev/null || true)" ]; then
        docker compose stop datahost >/dev/null 2>&1 || true
    fi
    docker volume create --label io.github.klanthier.vanillish=wowdata "$VOLUME" >/dev/null
    # The new archives land beside the old ones, which go only once the copy is whole.
    avail="$(docker run --rm -v "$VOLUME":/d "$HELPER" df -Pk /d 2>/dev/null | awk 'NR == 2 { print $4 }' || true)"
    case "$avail" in '' | *[!0-9]*) avail=0 ;; esac
    if [ "$avail" -gt 0 ] && [ $((avail * 1024)) -lt "$TOTAL" ]; then
        die "Docker's disk has $(gb $((avail * 1024))) GB free, and the game files need $(gb "$TOTAL") GB.
      Make room (docker system prune), or give Docker more disk (Docker Desktop > Settings > Resources)."
    fi
    t0=$SECONDS
    # shellcheck disable=SC2086 # $flags and $names split on purpose: archive names have no spaces
    got="$(COPYFILE_DISABLE=1 tar -c $flags -f - -C "$data" $names |
        docker run --rm -i -v "$VOLUME":/d -e RENAMES="$renames" "$HELPER" sh -euc "$IMPORT_SH" |
        LC_ALL=C sort)" || die_read "$data"
    [ "$got" = "$want" ] || die "the copy in $VOLUME does not match $data (the sizes differ); run this again"
    secs=$((SECONDS - t0))
    [ "$secs" -gt 0 ] || secs=1
    info "copied and checked: $(gb "$TOTAL") GB in ${secs}s ($((TOTAL / 1000000 / secs)) MB/s)"
}

game_data() {
    local wdd list src data
    say "Your game files"
    wdd="$(env_get WOW_DATA_DIR)"
    if [ -n "$wdd" ]; then
        if [ -d "$wdd" ]; then
            info ".env sets WOW_DATA_DIR=$wdd: the server reads the archives from that folder (no copy)"
            return 0
        fi
        warn ".env sets WOW_DATA_DIR=$wdd, which is not a folder here: the Docker volume is used instead"
        env_comment WOW_DATA_DIR
    fi
    list=""
    if docker_ok; then list="$(volume_list)"; fi
    if [ -z "$WOW_SRC" ] && [ "$REIMPORT" = 0 ] && has_required "$list"; then
        info "already copied: $(printf '%s\n' "$list" | awk 'NF { n++; s += $2 } END { printf "%d archives, %.1f GB", n, s / 1e9 }') in the Docker volume $VOLUME"
        return 0
    fi
    src="$WOW_SRC"
    if [ -z "$src" ]; then src="$(choose_wow_folder)"; fi
    src="$(clean_path "$src")"
    if [ -z "$src" ]; then
        if [ "$DRY" = 1 ]; then
            note "(dry run: no game folder given (--data DIR), so no copy to show)"
            return 0
        fi
        die "no folder chosen. Run this again, or name it: --data \"/path/to/World of Warcraft\""
    fi
    [ -d "$src" ] || die "$src is not a folder"
    # A folder macOS keeps Terminal out of (Desktop, Documents, Downloads, an external drive) looks
    # empty to the search below: say so instead.
    ls "$src" >/dev/null || die_read "$src"
    data="$(find_data "${src%/}")" || die "no 1.12.1 Data folder (one with dbc.MPQ) in $src"
    if entry "$data" common.MPQ >/dev/null || entry "$data" expansion.MPQ >/dev/null; then
        die "$data belongs to a later World of Warcraft (2.x or newer); Vanillish needs 1.12.1 (build 5875)"
    fi
    plan_import "$data"
    info "$data: $COUNT archives, $(gb "$TOTAL") GB"
    check_language "$data"
    if [ "$REIMPORT" = 0 ] && [ -n "$list" ] && [ "$(plan_list)" = "$list" ]; then
        info "already copied, unchanged"
        return 0
    fi
    import_archives "$data"
}

# ---- 7. creature pathfinding (mmaps), opt-in ---------------------------------------------------

choose_mmaps() {
    local want="$MMAPS"
    if [ -z "$want" ] && [ -n "$(env_get EXTRACT_MMAPS)" ]; then return 0; fi
    if [ -z "$want" ]; then
        say "Creature pathfinding (optional)"
        info "Without it, monsters chase you in straight lines, through trees and up cliffs. It is"
        info "generated from your game files after the server is up, in the background: about 1.5 to 3"
        info "hours on an Apple M4 (longer on a slower machine). You can play meanwhile; the world"
        info "server uses it from its next restart."
        if ask_yn "Generate creature pathfinding" n; then want=1; else want=0; fi
    fi
    if [ -z "$(env_get EXTRACT_MMAPS)" ]; then
        printf '\n# Creature pathfinding (mmaps), 1 or 0: generated once, in the background (hours).\n' >>"$ENV_FILE"
    fi
    env_set EXTRACT_MMAPS "$want"
}
# has_marker NAME — the extract service finished that step (.inilla-maps, -vmaps, -mmaps).
has_marker() {
    docker volume inspect "${PROJECT}_serverdata" >/dev/null 2>&1 || return 1
    docker run --rm -v "${PROJECT}_serverdata":/s:ro "$HELPER" test -f "/s/$1"
}
has_mmaps() { has_marker .inilla-mmaps; }

# ---- 8. start, and follow it -------------------------------------------------------------------

svc_states() { docker compose ps -a --format '{{.Service}}|{{.State}}|{{.Health}}|{{.ExitCode}}' 2>/dev/null || true; }
svc_field() { printf '%s\n' "$1" | awk -F'|' -v s="$2" -v n="$3" '$1 == s { v = $n } END { print v }'; }
all_up() {
    [ "$(svc_field "$1" realmd 3)" = healthy ] && [ "$(svc_field "$1" mangosd 3)" = healthy ] &&
        [ "$(svc_field "$1" datahost 3)" = healthy ]
}
broken() { # a one-shot failed, or a service stopped or is unhealthy
    printf '%s\n' "$1" | awk -F'|' '
        ($1 == "db-init" || $1 == "extract") && $2 == "exited" && $4 != 0 { bad = 1 }
        ($1 == "db" || $1 == "realmd" || $1 == "mangosd" || $1 == "datahost") && ($2 == "exited" || $2 == "dead" || $3 == "unhealthy") { bad = 1 }
        END { exit !bad }'
}
ESC=$'\033'
last_log() { # the last line service $1 printed, without its "[role] " prefix
    docker compose logs --no-color --no-log-prefix --tail 1 "$1" 2>/dev/null |
        tr -d '\r' | sed -e 's/^\[[A-Za-z-]*\] //' -e 's/…/.../g' | sed -n '$p' || true
}
last_line() { # the last line of compose's own output in $1
    sed -e "s/$ESC\\[[0-9;]*[A-Za-z]//g" -e '/^[[:space:]]*$/d' -e 's/^[[:space:]]*//' "$1" 2>/dev/null | sed -n '$p' || true
}

phase_of() { # phase_of STATES LOG — sets PHASE and DETAIL: what the first start is doing now
    PHASE="" DETAIL=""
    # mangosd before extract: on a first start it waits for extract, and beside the pathfinding
    # generation (extract, for hours) its loading is the news.
    if [ "$(svc_field "$1" db-init 2)" = running ]; then
        PHASE="Setting up the database (about a minute, once)"
        DETAIL="$(last_log db-init)"
    elif [ "$(svc_field "$1" mangosd 2)" = running ] && [ "$(svc_field "$1" mangosd 3)" != healthy ]; then
        PHASE="Loading the world (1-2 minutes)"
    elif [ "$(svc_field "$1" extract 2)" = running ]; then
        PHASE="Extracting the server's maps from your game files (3-5 minutes, once)"
        DETAIL="$(last_log extract)"
        case "$DETAIL" in *mmaps*) PHASE="Generating creature pathfinding (hours, in the background)" ;; esac
    elif [ "$(svc_field "$1" datahost 2)" = running ] && [ "$(svc_field "$1" datahost 3)" != healthy ]; then
        PHASE="Indexing the game files for the phone"
    elif [ -n "$2" ]; then
        DETAIL="$(last_line "$2")"
        case "$DETAIL" in
            '#'* | *[Bb]uild*) PHASE="Building the server's images here: no prebuilt one could be downloaded (10-20 minutes)" ;;
            *) PHASE="Starting" ;;
        esac
    else
        PHASE="Starting"
    fi
}

LINE_OPEN=0
end_line() {
    if [ "$LINE_OPEN" = 1 ]; then
        printf '\n'
        LINE_OPEN=0
    fi
}
show_progress() {
    local line el
    if [ "$PHASE" != "$LASTPHASE" ]; then end_line; fi
    el="$(mmss $((SECONDS - WATCH_T0)))"
    line="$PHASE${DETAIL:+: $DETAIL}"
    if [ "$TTY" = 1 ]; then
        printf '\r%s[K    %s  %s' "$ESC" "$el" "$(printf '%s' "$line" | cut -c1-"$COLS")"
        LINE_OPEN=1
    elif [ "$line" != "$LASTLINE" ] || [ $((SECONDS - BEAT)) -ge 60 ]; then
        printf '    %s  %s\n' "$el" "$line"
        BEAT=$SECONDS
    fi
    LASTPHASE="$PHASE" LASTLINE="$line"
}

# watch_start HOW [PID [LOG [SECONDS]]] — the start's progress, until:
#   up PID      this run's background `up` (PID) exits; then its status
#   other PID   another run's `up` (PID) exits
#   extract     the extract service stops running (1 when the time is over)
#   ready       the realm is up (1 when something failed or the time is over)
#   restart     the same, after a restart of mangosd
# Only a first start tells the player the phone can begin (a restart is no news to them).
watch_start() {
    local how="$1" pid="${2:-}" log="${3:-}" max="${4:-3600}" st rc=0 told=0
    case "$how" in ready | restart) told=1 ;; esac
    LASTPHASE="" LASTLINE="" WATCH_T0=$SECONDS BEAT=$SECONDS
    COLS="$(tput cols 2>/dev/null || echo 100)"
    case "$COLS" in '' | *[!0-9]*) COLS=100 ;; esac
    COLS=$((COLS > 30 ? COLS - 16 : 30))
    while :; do
        case "$how" in up | other) kill -0 "$pid" 2>/dev/null || break ;; esac
        st="$(svc_states)"
        case "$how" in
            extract) [ "$(svc_field "$st" extract 2)" = running ] || break ;;
            ready | restart)
                if all_up "$st"; then break; fi
                if broken "$st"; then
                    rc=1
                    break
                fi
                ;;
        esac
        # This run's `up` has its own --wait-timeout (a local image build comes before it).
        if [ "$how" != up ] && [ $((SECONDS - WATCH_T0)) -ge "$max" ]; then
            rc=1
            break
        fi
        phase_of "$st" "$log"
        show_progress
        if [ "$told" = 0 ] && [ "$(svc_field "$st" datahost 3)" = healthy ] && [ "$(svc_field "$st" mangosd 3)" != healthy ]; then
            end_line
            info "${B}The phone can start already:${N} in Vanillish, type $(typed "$IP") to download the game files"
            info "and make your account while the server finishes starting."
            told=1
        fi
        sleep 3
    done
    end_line
    if [ "$how" = up ]; then wait "$pid" || rc=$?; fi
    return "$rc"
}

diagnose() {
    local st s state health code
    st="$(svc_states)"
    for s in db db-init extract realmd mangosd datahost; do
        state="$(svc_field "$st" "$s" 2)"
        health="$(svc_field "$st" "$s" 3)"
        code="$(svc_field "$st" "$s" 4)"
        if { [ "$state" = exited ] && [ "$code" != 0 ]; } || [ "$health" = unhealthy ] || [ "$state" = restarting ]; then
            warn "$s: $state${health:+ ($health)}${code:+, exit $code}. Its last lines:"
            docker compose logs --no-color --tail 25 "$s" >&2 || true
        fi
    done
}

# shellcheck disable=SC2329 # the INT trap calls it
on_interrupt() {
    end_line
    if [ -n "${UPPID:-}" ] && kill -0 "$UPPID" 2>/dev/null; then
        warn "stopped following: the server goes on starting by itself. To see where it is:"
        warn "  cd $(disp_dir) && docker compose ps"
    fi
    exit 130
}

# The `up` an earlier run left going (it outlives a closed window and Ctrl-C), if it still runs:
# its PID. Its command line is checked too, since a PID is reused after a restart. One file per
# user and stack (Linux has one /tmp for every user).
UP_PIDFILE="${TMPDIR:-/tmp}/vanillish-up-$(id -u)-$PROJECT.pid"
earlier_up() {
    local pid
    pid="$(cat "$UP_PIDFILE" 2>/dev/null || true)"
    case "$pid" in '' | 0 | 1 | *[!0-9]*) return 1 ;; esac
    kill -0 "$pid" 2>/dev/null || return 1
    case "$(ps -p "$pid" -o args= 2>/dev/null || true)" in
        *compose*up*)
            printf '%s' "$pid"
            return 0
            ;;
    esac
    return 1
}
# extract_env NAME — that variable in the running extract container ("" when none runs).
extract_env() {
    local id
    id="$(docker compose ps -q --status running extract 2>/dev/null | sed -n 1p || true)"
    [ -n "$id" ] || return 0
    docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$id" 2>/dev/null |
        sed -n "s/^$1=//p" | sed -n 1p || true
}

start_stack() {
    local st later_mm log pid rc=0
    say "Starting the server"
    if [ "$DRY" = 1 ]; then
        note "(dry run: would run: docker compose pull, then docker compose up -d --wait, and follow it)"
        return 0
    fi
    docker compose config -q || die "compose.yaml and .env do not fit together (above)"
    # Two `up`s at once trip over each other: an earlier run's is followed to its end first, and
    # this run's own start below then applies this run's .env (a new address) and finds the rest done.
    if pid="$(earlier_up)"; then
        info "an earlier start is still under way; following it to its end first"
        watch_start other "$pid" "" 3600 || true
    fi
    rm -f "$UP_PIDFILE"
    st="$(svc_states)"
    if [ "$(svc_field "$st" extract 2)" = running ]; then
        if [ "$(extract_env EXTRACT_MMAPS)" = 1 ] && ! has_mmaps; then
            # Hours of pathfinding, in the background: the realm runs beside it, and an `up` would
            # wait for its end (mangosd waits for extract). Only what is not running is started,
            # once the maps it needs are there (the pathfinding comes after them).
            info "creature pathfinding is still being generated; the realm runs meanwhile."
            info "(an update waits until that is done: run this again then)"
            if all_up "$st"; then
                :
            elif has_marker .inilla-vmaps; then
                docker compose up -d --no-deps db realmd mangosd datahost >/dev/null 2>&1 ||
                    warn "could not start the realm beside it: docker compose ps"
                watch_start ready "" "" 900 || warn "the realm is slow to come up: docker compose ps"
            else
                warn "the world server waits for the map extraction: run this again in a few minutes"
            fi
            return 0
        fi
        # The first start's map extraction, which nothing follows any more (an interrupted `up`):
        # once it is done, the start below brings up the world server that waits for it.
        info "the server's maps are still being extracted from an earlier start; waiting for that first"
        watch_start extract "" "" 3600 || die "the map extraction does not end: docker compose logs extract"
    fi
    # iNilla's server, the stack this one grew from, wants the same ports.
    if [ -n "$(docker ps -q --filter label=com.docker.compose.project=inilla 2>/dev/null || true)" ]; then
        warn "an iNilla server runs here and holds ports 3724, 8085 and 8099: stop it first"
        warn "  (in its folder: docker compose stop)"
    fi
    info "downloading the images (about 1 GB the first time)"
    if [ "$TTY" = 1 ]; then
        docker compose pull || warn "an image could not be downloaded: it is built here instead (10-20 minutes)"
    else
        docker compose pull --quiet || warn "an image could not be downloaded: it is built here instead (10-20 minutes)"
    fi
    # `up` runs on its own, so closing this window or Ctrl-C only stops the following: compose
    # still starts mangosd once its maps are there. nohup is for the closed window. Ctrl-C reaches
    # the terminal's whole foreground process group, and a background job's ignored SIGINT does not
    # hold (the docker CLI listens for it again), so the launch puts `up` in a group of its own:
    # job control (set -m) on for that one command.
    log="$TMP/up.log"
    # Pathfinding asked for and not made yet: the world first (this start without it), and the
    # pathfinding follows in the background (start_mmaps).
    later_mm="$(env_get EXTRACT_MMAPS)"
    if [ "$later_mm" = 1 ] && has_mmaps; then later_mm=0; fi
    set -m
    if [ "$later_mm" = 1 ]; then
        EXTRACT_MMAPS=0 nohup docker compose up -d --wait --wait-timeout 3600 >"$log" 2>&1 </dev/null &
    else
        nohup docker compose up -d --wait --wait-timeout 3600 >"$log" 2>&1 </dev/null &
    fi
    UPPID=$!
    set +m
    printf '%s\n' "$UPPID" >"$UP_PIDFILE"
    watch_start up "$UPPID" "$log" || rc=$?
    UPPID=""
    rm -f "$UP_PIDFILE"
    if [ "$rc" != 0 ]; then
        sed -n '$p' "$log" >&2 || true
        diagnose
        die "the server did not come up. Its logs: cd $(disp_dir) && docker compose logs"
    fi
    info "${G}up${N}: realmd (logon), mangosd (the world) and the data host all answer"
}

start_mmaps() {
    if [ "$DRY" = 1 ] || [ "$(env_get EXTRACT_MMAPS)" != 1 ]; then return 0; fi
    if has_mmaps; then
        # mangosd turns mmaps on only when it starts with them on disk.
        if docker compose exec -T mangosd grep -qi '^mmap.enabled = 0' /run/vmangos/mangosd.conf 2>/dev/null; then
            info "creature pathfinding is ready: restarting the world server to use it (characters are saved)"
            docker compose restart mangosd >/dev/null 2>&1 || true
            watch_start restart "" "" 600 || warn "the world server is slow to come back: docker compose logs mangosd"
        fi
        return 0
    fi
    if [ -z "$(docker compose ps -q --status running extract 2>/dev/null || true)" ]; then
        if ! docker compose up -d --no-deps extract >/dev/null 2>&1; then
            warn "could not start generating creature pathfinding: docker compose logs extract"
            return 0
        fi
    fi
    MMAPS_RUNNING=1
}

# ---- 9. reachability, staying up ---------------------------------------------------------------

keep() { KEEP="$KEEP  - $*$NL"; }

# firewall_blocks_all GLOBALSTATE BLOCKALL — what socketfilterfw --getglobalstate and --getblockall
# printed says the firewall is on and set to "Block all incoming connections". The wording differs
# between macOS versions ("Block all ENABLED!", "... block all state set to enabled."), so only the
# words count, in any case; state 2 is that setting itself.
firewall_blocks_all() {
    local state all
    state="$(lower "$1")"
    all="$(lower "$2")"
    case "$state" in
        *"state = 2"*) return 0 ;;
        *disabled* | *"state = 0"*) return 1 ;;
    esac
    case "$all" in
        *disabled*) return 1 ;;
        *enabled*) return 0 ;;
    esac
    return 1
}

# is_laptop BATT MODEL — what pmset -g batt and sysctl hw.model printed says this Mac has a battery.
# Apple-silicon MacBooks since the M2 are "MacNN,N" models, so the battery decides.
is_laptop() {
    case "$1" in *InternalBattery*) return 0 ;; esac
    case "$2" in *MacBook*) return 0 ;; esac
    return 1
}

after_checks() {
    local fw=/usr/libexec/ApplicationFirewall/socketfilterfw store
    KEEP=""
    if [ "$DRY" = 0 ]; then
        if curl -fsS --max-time 5 "http://$IP:$PORT/server.json" >"$TMP/server.json" 2>/dev/null &&
            grep -q '"format"' "$TMP/server.json"; then
            :
        elif curl -fsS --max-time 5 "http://$IP:$PORT/" >/dev/null 2>&1; then
            warn "the data host answers at http://$IP:$PORT/ but has no /server.json: the app's quick connect needs a newer image"
        else
            warn "nothing answers at http://$IP:$PORT/ from this machine itself, so the phone cannot reach it either."
            if [ "$OS" = linux ] && [ "$WSL" = 0 ]; then warn "  a firewall? (ufw: sudo ufw allow 3724,8085,8099/tcp)"; fi
            if [ "$WSL" = 1 ]; then warn "  Windows' firewall must let 'Docker Desktop Backend' accept connections on private networks"; fi
        fi
    fi
    if [ "$OS" = linux ]; then
        if [ "$WSL" = 1 ]; then
            keep "Docker Desktop must start with Windows (Settings > General > Start Docker Desktop when you sign in); the server then starts by itself."
        else
            keep "The server starts by itself with Docker: sudo systemctl enable docker"
        fi
        return 0
    fi
    if [ -x "$fw" ] && firewall_blocks_all "$(mac "$fw" --getglobalstate 2>/dev/null || true)" \
        "$(mac "$fw" --getblockall 2>/dev/null || true)"; then
        warn "the macOS firewall blocks ALL incoming connections, so the phone cannot connect:"
        warn "  System Settings > Network > Firewall > Options > turn off 'Block all incoming connections'"
    fi
    case "$RUNTIME" in
        desktop)
            store="$HOME/Library/Group Containers/group.com.docker/settings-store.json"
            if [ "$(mac plutil -extract AutoStart raw -o - "$store" 2>/dev/null || echo false)" != true ]; then
                keep "Docker Desktop must start when you log in, or the server stays off after a restart:
    Docker Desktop > Settings > General > 'Start Docker Desktop when you sign in to your computer'."
            fi
            ;;
        orbstack) keep "OrbStack must start when you log in (OrbStack > Settings > 'Start at login'); the server then starts by itself." ;;
        colima) keep "Colima must start when you log in: brew services start colima" ;;
    esac
    if mac pmset -g 2>/dev/null | awk '$1 == "sleep" && $2 != 0 { f = 1 } END { exit !f }'; then
        keep "This Mac must stay awake while you play: System Settings > Energy > 'Prevent automatic sleeping
    when the display is off' (a laptop: Battery > Options), or keep 'caffeinate -i' running in a Terminal."
    fi
    if is_laptop "$(mac pmset -g batt 2>/dev/null || true)" "$(mac sysctl -n hw.model 2>/dev/null || true)"; then
        keep "A laptop: on another Wi-Fi (a café, a hotel) anyone there could download your game files from it.
    Away from home: cd $(disp_dir) && docker compose stop"
    fi
}

# ---- 10. the summary ---------------------------------------------------------------------------

box() { # box LINE… — a frame around the lines (ASCII inside, so the width counts right)
    local l w=0 bar="" i h v tl tr bl br
    for l in "$@"; do if [ "${#l}" -gt "$w" ]; then w=${#l}; fi; done
    w=$((w + 6))
    case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
        *[Uu][Tt][Ff]-8* | *[Uu][Tt][Ff]8*) h='═' v='║' tl='╔' tr='╗' bl='╚' br='╝' ;;
        *) h='=' v='|' tl='+' tr='+' bl='+' br='+' ;;
    esac
    i=0
    while [ "$i" -lt "$w" ]; do
        bar="$bar$h"
        i=$((i + 1))
    done
    printf '\n  %s%s%s%s%s\n' "$G" "$tl" "$bar" "$tr" "$N"
    for l in "$@"; do printf '  %s%s%s   %s%-*s%s   %s%s%s\n' "$G" "$v" "$N" "$B" $((w - 6)) "$l" "$N" "$G" "$v" "$N"; done
    printf '  %s%s%s%s%s\n' "$G" "$bl" "$bar" "$br" "$N"
}

summary() {
    local dd setup also=""
    dd="$(disp_dir)"
    if [ "$OS" = mac ]; then setup="$dd/setup-mac.sh"; else setup="$dd/setup-linux.sh"; fi
    if [ -n "$LOCALNAME" ]; then also="      (or $(typed "$LOCALNAME.local"))"; fi
    if [ "$DRY" = 1 ]; then say "Dry run done: nothing was changed. A real run ends with:"; fi
    set -- "" "Your Vanillish server is running." "" "On your phone, open Vanillish and type:" "" "      $(typed "$IP")"
    if [ -n "$also" ]; then set -- "$@" "$also"; fi
    box "$@" "" \
        "The phone downloads the game files from here once" \
        "(about 5 GB, on the same Wi-Fi), you make your account" \
        "in the app, and you play." ""
    if [ "${MMAPS_RUNNING:-0}" = 1 ]; then
        printf '\n  Creature pathfinding is being generated in the background (about 1.5-3 hours on an M4).\n'
        printf '  The world server uses it from its next restart: run the setup again then, or\n'
        printf '  cd %s && docker compose restart mangosd\n' "$dd"
    fi
    printf '\n  Accounts are made in the app (Create account), from phones on your network. To turn that\n'
    printf '  off: INILLA_SIGNUP=off in .env, then run the setup again. A game master: see below.\n'
    if [ -n "$KEEP" ]; then
        printf '\n  %sTo keep it reachable:%s\n%s' "$B" "$N" "$KEEP"
    fi
    cat <<EOF

  ${B}Everyday${N} (in a Terminal):
    stop the server           cd $dd && docker compose stop
    start it again            cd $dd && docker compose start
    update it, or after the
    address changed           $setup
    make an account a GM      cd $dd && docker compose exec mangosd vmangos-console account set gmlevel NAME 6
    the world server's log    cd $dd && docker compose logs -f mangosd
    remove everything         $setup --uninstall

  Settings: $dd/.env and $dd/config/. More: $dd/README.md
EOF
}

# ---- uninstall ---------------------------------------------------------------------------------

uninstall() {
    local rmi=""
    say "Uninstalling the Vanillish server in $(disp_dir)"
    [ -f "$DIR/compose.yaml" ] || die "no Vanillish server in $DIR"
    if ! docker info >/dev/null 2>&1; then die "Docker is not running: start it, then run this again"; fi
    cd "$DIR"
    ask_yn "Stop the server and delete its database (accounts, characters) and map data" n || die "nothing was changed"
    if ask_yn "Also delete its Docker images (about 1 GB; downloaded again by a new setup)" y; then rmi="--rmi all"; fi
    # shellcheck disable=SC2086 # $rmi is empty or two words
    act docker compose down -v --remove-orphans $rmi
    if docker volume inspect "$VOLUME" >/dev/null 2>&1 &&
        ask_yn "Delete your copied game files too (the Docker volume $VOLUME, about 5 GB)" y; then
        act docker volume rm "$VOLUME" >/dev/null
    fi
    if [ "$MODE" = installed ] && ask_yn "Delete the folder $(disp_dir) (its .env holds the passwords)" y; then
        cd /
        act rm -rf "$DIR"
    fi
    say "Done."
}

# ---- main --------------------------------------------------------------------------------------

main() {
    parse_args "$@"
    detect_platform
    locate_dir
    # Docker Desktop and OrbStack put their CLI here when /usr/local/bin was not writable.
    PATH="$PATH:/usr/local/bin:/opt/homebrew/bin:$HOME/.docker/bin:$HOME/.orbstack/bin:/Applications/Docker.app/Contents/Resources/bin"
    export PATH
    TMP="$(mktemp -d "${TMPDIR:-/tmp}/vanillish-setup.XXXXXX")"
    mkdir "$TMP/empty"
    trap 'rm -rf "$TMP"' EXIT
    trap on_interrupt INT
    DOCKER_OK=0 RUNTIME="" LOCALNAME="" KEEP="" STAGE="" UPPID="" PORT=8099
    if [ "$UNINSTALL" = 1 ]; then
        uninstall
        return 0
    fi
    [ "$DRY" = 0 ] || say "Dry run: nothing on this machine changes"
    if [ "$MODE" != checkout ] && { [ "$NO_UPDATE" = 0 ] || [ ! -f "$DIR/compose.yaml" ]; }; then
        fetch_stack
        # An installed copy hands over to the one it just downloaded, which may know more.
        if [ "$DRY" = 0 ] && [ -n "$SELF_FILE" ] && [ "$SELF_FILE" = "$DIR/setup-mac.sh" ]; then
            rm -rf "$TMP"
            export VANILLISH_UPDATED=1
            exec "${BASH:-/bin/bash}" "$DIR/setup-mac.sh" --no-update "$@"
        fi
    elif [ "${VANILLISH_UPDATED:-0}" = 0 ]; then
        say "Server files: $(disp_dir) (as they are)"
    fi
    check_machine
    check_docker
    if [ "$DRY" = 1 ] && [ ! -d "$DIR" ]; then cd "$STAGE"; else cd "$DIR"; fi
    ENV_FILE="$DIR/.env"
    if [ "$DRY" = 1 ]; then
        # A rehearsal writes its .env beside the temporary files and shows it at the end.
        if [ -f "$DIR/.env" ]; then cp "$DIR/.env" "$TMP/env"; fi
        ENV_FILE="$TMP/env"
    fi
    find_ip
    write_env
    game_data
    choose_mmaps
    start_stack
    start_mmaps
    after_checks
    if [ "$DRY" = 1 ]; then
        say "The .env it would write (passwords hidden)"
        sed -e 's/^\([A-Z_]*PASSWORD\)=.*/\1=…/' "$ENV_FILE" | sed 's/^/    /'
    fi
    summary
}

# ci/setup-test.sh sources this file for its functions, and nothing runs.
if [ "${VANILLISH_SETUP_LIB:-0}" = 1 ]; then return 0; fi
main "$@"; exit $?
