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
    # Asked nothing: a question here would read the rest of this script as its answer.
    apt-get update -qq </dev/null
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
        ca-certificates curl openssl coreutils </dev/null >/dev/null
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
# Checking the signature takes OpenSSL 3: an older one fails on a sound release.
HAS="$(openssl version 2>/dev/null | awk '{print $2}')"
case "$HAS" in
    [3-9].* | [1-9][0-9].*) ;;
    *) refuse "this machine has OpenSSL ${HAS:-of an unknown version}, which cannot check Orbite's signature. It takes OpenSSL 3 or later, as on Debian 12 and Ubuntu 22.04." ;;
esac
printf '%s\n' "$KEY" >"$WORK/orbite.pub"
openssl pkeyutl -verify -pubin -inkey "$WORK/orbite.pub" -rawin \
    -in "$WORK/SHA256SUMS" -sigfile "$WORK/SHA256SUMS.sig" >/dev/null 2>&1 ||
    refuse "what was downloaded is not signed by Orbite's key."
(cd "$WORK" && grep " $FILE\$" SHA256SUMS | sha256sum -c - >/dev/null 2>&1) ||
    refuse "what was downloaded does not match its signed checksum."

# An older release is not put over a newer Orbite: a release that was once
# sound stays signed for ever, known defects included.
if [ -x "$ROOT$PROGRAM" ] && [ "${ORBITE_ALLOW_OLDER:-}" != 1 ] &&
    curl -fsSL "$RELEASE/VERSION" -o "$WORK/VERSION" 2>/dev/null &&
    (cd "$WORK" && grep ' VERSION$' SHA256SUMS | sha256sum -c - >/dev/null 2>&1); then
    OFFERED="$(tr -d ' \n' <"$WORK/VERSION")"
    INSTALLED="$("$ROOT$PROGRAM" version 2>/dev/null | awk '{print $2}')"
    if [ -n "$INSTALLED" ] && [ "$INSTALLED" != "$OFFERED" ] &&
        [ "$(printf '%s\n%s\n' "$INSTALLED" "$OFFERED" | sort -V | tail -1)" = "$INSTALLED" ]; then
        refuse "this machine runs Orbite $INSTALLED, and what was downloaded is the older $OFFERED. To install it all the same, set ORBITE_ALLOW_OLDER=1."
    fi
fi

# --- Now the machine is changed ---

UPDATING=no
[ -f "$ROOT$SERVICE" ] && UPDATING=yes

if ! id orbite >/dev/null 2>&1; then
    useradd --system --home-dir "$HOME_DIR" --shell /usr/sbin/nologin orbite
fi
mkdir -p "$ROOT$HOME_DIR"
# With all that is in it: data kept from an earlier installation was given
# to root when that one was removed.
chown -R orbite: "$ROOT$HOME_DIR"
chmod 700 "$ROOT$HOME_DIR"

mkdir -p "$ROOT$(dirname "$PROGRAM")"
cp "$WORK/$FILE" "$ROOT$PROGRAM.new"
chmod 755 "$ROOT$PROGRAM.new"
mv "$ROOT$PROGRAM.new" "$ROOT$PROGRAM"

mkdir -p "$ROOT$(dirname "$SERVICE")"
cat >"$WORK/orbite.service" <<UNIT
# Written by Orbite's installer, and written again each time it runs.
# Changes of your own belong in a drop-in, which is kept: systemctl edit orbite
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
# A service file its Owner changed is not lost without a word.
grep -v '^#' "$WORK/orbite.service" >"$WORK/service.new"
[ -f "$ROOT$SERVICE" ] && grep -v '^#' "$ROOT$SERVICE" >"$WORK/service.old"
if [ -f "$ROOT$SERVICE" ] && ! cmp -s "$WORK/service.new" "$WORK/service.old"; then
    cp "$ROOT$SERVICE" "$ROOT$SERVICE.replaced"
    say "The service file had been changed: it is written anew, and the one before is kept as $SERVICE.replaced. Put changes of your own in a drop-in, which is kept: systemctl edit orbite"
fi
cp "$WORK/orbite.service" "$ROOT$SERVICE"
chmod 644 "$ROOT$SERVICE"

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

# --- The Agent's Computer: a screen of its own, a browser, and fonts ---
#
# The browser is Firefox from Mozilla's own packages: the only one packaged
# for every machine Orbite installs on (Ubuntu ships browsers as snaps, which
# a service account cannot run). ORBITE_WITHOUT_COMPUTER=1 leaves all of this
# out. Orbite is installed whether or not this part succeeds.

MOZILLA_KEY=35BAA0B33E9EB396F59CA838C0BA5CE6DC6315A3

computer() {
    mkdir -p "$ROOT/etc/apt/keyrings" "$ROOT/etc/apt/sources.list.d" "$ROOT/etc/apt/preferences.d"
    KEYRING=/etc/apt/keyrings/packages.mozilla.org.asc
    if [ ! -s "$ROOT$KEYRING" ]; then
        curl -fsSL "${ORBITE_MOZILLA_KEY_URL:-https://packages.mozilla.org/apt/repo-signing-key.gpg}" \
            -o "$ROOT$KEYRING.new" || return 1
        # Mozilla publishes the fingerprint of its key: it is checked where
        # the machine has what it takes, and the key is refused if it differs.
        if command -v gpg >/dev/null 2>&1; then
            mkdir -p -m 700 "$WORK/gnupg"
            gpg --homedir "$WORK/gnupg" -n -q --import --import-options import-show --with-colons \
                "$ROOT$KEYRING.new" 2>/dev/null |
                grep -q "^fpr:.*:$MOZILLA_KEY:" || {
                rm -f "$ROOT$KEYRING.new"
                say "The key offered for Mozilla's packages is not Mozilla's."
                return 1
            }
        fi
        mv "$ROOT$KEYRING.new" "$ROOT$KEYRING"
    fi
    echo "deb [signed-by=$KEYRING] https://packages.mozilla.org/apt mozilla main" \
        >"$ROOT/etc/apt/sources.list.d/mozilla.list"
    # Only the browser is taken from there, whatever else that source offers.
    printf 'Package: firefox*\nPin: origin packages.mozilla.org\nPin-Priority: 1000\n\nPackage: *\nPin: origin packages.mozilla.org\nPin-Priority: 1\n' \
        >"$ROOT/etc/apt/preferences.d/mozilla"
    apt-get update -qq </dev/null || return 1
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends --allow-downgrades \
        xvfb xdotool firefox fonts-liberation fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji \
        </dev/null >/dev/null || return 1
}

if [ "${ORBITE_WITHOUT_COMPUTER:-}" = "1" ]; then
    say "Left out, as asked: the agent's own computer. Without it the agent cannot use sites that need"
    say "signing in or a person's gestures. Run this installer again without ORBITE_WITHOUT_COMPUTER to add it."
else
    say "Setting up the agent's own computer (a screen, Firefox and fonts)..."
    if ! computer; then
        say "Note: the agent's computer could not be set up. Orbite works without it; run this installer"
        say "again later to add it."
    fi
fi

if [ "$UPDATING" = yes ]; then
    # An Orbite that was never set up has nothing to run yet.
    if [ -f "$ROOT$HOME_DIR/config.json" ]; then
        systemctl restart orbite
        say "Orbite is updated and running."
    else
        say "Orbite is updated. Set it up with: sudo orbite setup"
    fi
    exit 0
fi

if [ "${ORBITE_SKIP_SETUP:-}" = "1" ]; then
    say "Orbite is installed. Set it up with: sudo orbite setup"
    exit 0
fi

# Setup asks its questions in this terminal, even when the installer itself
# was piped in.
say ""
runuser -u orbite -- env ORBITE_HOME="$HOME_DIR" "$PROGRAM" setup </dev/tty
systemctl start orbite
say "Orbite is running, and will start again by itself after a restart."
