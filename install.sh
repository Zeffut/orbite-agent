#!/bin/sh
# Installs Orbite on this machine: the program, an account of its own, and a
# service that restarts by itself. Nothing is changed before the machine has
# been checked, and nothing is installed that is not signed by Orbite's key.
#
#   curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
#
# Running it again updates the program and keeps everything else.
set -eu

# For trying the installer out: another root folder, another place to
# download from, another key, no setup at the end.
ROOT="${ORBITE_ROOT:-}"
RELEASE="${ORBITE_RELEASE_URL:-https://github.com/Zeffut/orbite-agent/releases/latest/download}"
KEY="${ORBITE_SIGNING_KEY:------BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEA6FY16XGsLI6MBCwCVPfwbDc4H2bhT+/LMojxS4Q/NEg=
-----END PUBLIC KEY-----}"

HOME_DIR=/var/lib/orbite
PROGRAM=/usr/local/bin/orbite
SERVICE=/etc/systemd/system/orbite.service

say() { printf '%s\n' "$*"; }
refuse() {
    printf 'Orbite was not installed: %s\n' "$*" >&2
    exit 1
}

# --- The machine is checked before anything is changed ---

[ "$(uname -s)" = "Linux" ] || refuse "this installer is for Linux."

case "$(uname -m)" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) refuse "processors of the kind $(uname -m) are not supported (x86_64 and aarch64 are)." ;;
esac

ID=unknown
PRETTY_NAME="This system"
# shellcheck disable=SC1091
[ -r "$ROOT/etc/os-release" ] && . "$ROOT/etc/os-release"
case "$ID" in
    debian | ubuntu) ;;
    *) refuse "$PRETTY_NAME is not supported yet (Debian and Ubuntu are)." ;;
esac

[ -d "$ROOT/run/systemd/system" ] || refuse "systemd is not running here, and Orbite runs as a systemd service."

[ "$(id -u)" = "0" ] || refuse "it must be run as root, for example: curl ... | sudo sh"

MEMORY_KB=$(awk '/^MemTotal:/ { print $2 }' "$ROOT/proc/meminfo" 2>/dev/null || true)
if [ -n "${MEMORY_KB:-}" ] && [ "$MEMORY_KB" -lt 1900000 ]; then
    say "Note: this machine has little memory ($((MEMORY_KB / 1024)) MB). Orbite will run, but the"
    say "agent's own computer (its desktop and browser) needs about 2 GB to be comfortable."
fi

# --- What the installer itself needs ---

MISSING=""
for tool in curl openssl sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || MISSING="$MISSING $tool"
done
if [ -n "$MISSING" ]; then
    say "Installing what the installer needs:$MISSING"
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends ca-certificates curl openssl coreutils >/dev/null
fi

# --- The program is downloaded and checked before it is put anywhere ---

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FILE="orbite-$ARCH-linux"

say "Downloading Orbite for $ARCH..."
for name in "$FILE" SHA256SUMS SHA256SUMS.sig; do
    curl -fsSL "$RELEASE/$name" -o "$WORK/$name" || refuse "$name could not be downloaded from $RELEASE."
done

# The list of checksums is signed; the program must match its checksum.
printf '%s\n' "$KEY" >"$WORK/orbite.pub"
openssl pkeyutl -verify -pubin -inkey "$WORK/orbite.pub" -rawin \
    -in "$WORK/SHA256SUMS" -sigfile "$WORK/SHA256SUMS.sig" >/dev/null 2>&1 ||
    refuse "what was downloaded is not signed by Orbite's key."
(cd "$WORK" && grep " $FILE\$" SHA256SUMS | sha256sum -c - >/dev/null 2>&1) ||
    refuse "what was downloaded does not match its signed checksum."

# --- Now the machine is changed ---

UPDATING=no
[ -f "$ROOT$SERVICE" ] && UPDATING=yes

if ! id orbite >/dev/null 2>&1; then
    useradd --system --home-dir "$HOME_DIR" --shell /usr/sbin/nologin orbite
fi
mkdir -p "$ROOT$HOME_DIR"
chown orbite: "$ROOT$HOME_DIR"
chmod 700 "$ROOT$HOME_DIR"

mkdir -p "$ROOT$(dirname "$PROGRAM")"
cp "$WORK/$FILE" "$ROOT$PROGRAM.new"
chmod 755 "$ROOT$PROGRAM.new"
mv "$ROOT$PROGRAM.new" "$ROOT$PROGRAM"

mkdir -p "$ROOT$(dirname "$SERVICE")"
cat >"$ROOT$SERVICE" <<UNIT
[Unit]
Description=Orbite, a personal AI agent
Documentation=https://github.com/Zeffut/orbite-agent
After=network-online.target
Wants=network-online.target

[Service]
User=orbite
Group=orbite
WorkingDirectory=$HOME_DIR
Environment=ORBITE_HOME=$HOME_DIR
ExecStart=$PROGRAM run
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT

# Orbite keeps itself up to date: once a day it installs the latest release
# signed by its key, and goes back to the one before if that does not start.
# The Owner turns it off with: orbite update off
cat >"$ROOT/etc/systemd/system/orbite-update.service" <<UNIT
[Unit]
Description=Update Orbite to the latest signed release
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
Environment=ORBITE_HOME=$HOME_DIR
ExecStart=$PROGRAM update
UNIT
cat >"$ROOT/etc/systemd/system/orbite-update.timer" <<UNIT
[Unit]
Description=Update Orbite once a day

[Timer]
OnCalendar=daily
RandomizedDelaySec=6h
Persistent=true

[Install]
WantedBy=timers.target
UNIT

systemctl daemon-reload
systemctl enable orbite >/dev/null 2>&1 || systemctl enable orbite
systemctl enable --now orbite-update.timer >/dev/null 2>&1 || systemctl enable --now orbite-update.timer

if [ "$UPDATING" = yes ]; then
    systemctl restart orbite
    say "Orbite is updated and running."
    exit 0
fi

if [ "${ORBITE_SKIP_SETUP:-}" = "1" ]; then
    say "Orbite is installed. Set it up with: sudo -u orbite ORBITE_HOME=$HOME_DIR orbite setup"
    exit 0
fi

# Setup asks its questions in this terminal, even when the installer itself
# was piped in.
say ""
runuser -u orbite -- env ORBITE_HOME="$HOME_DIR" "$PROGRAM" setup </dev/tty
systemctl start orbite
say "Orbite is running, and will start again by itself after a restart."
