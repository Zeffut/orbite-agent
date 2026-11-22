#!/bin/sh
# Installs Orbite on this machine: the program, an account of its own, and a
# service that restarts by itself. Nothing is changed before the machine has
# been checked, and nothing is installed that is not signed by Orbite's key.
#
#   curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
#
# or, where there is no curl:
#
#   wget -qO- https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
#
# It runs on any Linux with systemd, freshly installed, with nothing to
# prepare: what it needs and the system lacks, it installs itself.
#
# Running it again updates the program and keeps everything else.
set -eu
# What is installed for the whole machine is readable by it, however
# strictly the account running this makes its own files: a key that apt
# cannot read is a key apt does not trust. The Agent's folder is closed
# to others further down, by name.
umask 022

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
# What the installer is doing, to say where it stopped.
STEP="checking the machine"
# Yes when a failure is one the installer did not foresee.
UNFORESEEN=yes
refuse() {
    UNFORESEEN=no
    printf 'Orbite was not installed: %s\n' "$*" >&2
    exit 1
}

# `$1` as it is written in an address: every byte by its number, then
# letters and digits put back as they are. With nothing but what the
# smallest system has: no awk there, on some.
in_an_address() {
    UNWRITE=""
    for letter in a b c d e f g h i j k l m n o p q r s t u v w x y z \
        A B C D E F G H I J K L M N O P Q R S T U V W X Y Z 0 1 2 3 4 5 6 7 8 9; do
        UNWRITE="$UNWRITE;s/%$(printf '%x' "'$letter")/$letter/g"
    done
    printf '%s' "$1" | od -An -v -tx1 | tr -d ' \n' | sed 's/../%&/g' | sed "$UNWRITE"
}

# When the installer stops on something it did not foresee, it offers a
# report: a link to a new issue, filled in with what went wrong and on what
# kind of system. Nothing is sent; the Owner opens the link, or does not.
offer_a_report() {
    code=$?
    rm -rf "${WORK:-}"
    [ "$code" -ne 0 ] && [ "$UNFORESEEN" = yes ] || exit "$code"
    printf '\nOrbite was not installed: the installer stopped while %s.\n' "$STEP" >&2
    report "The installer stopped while $STEP (exit code $code)." "Install failed while $STEP"
    exit "$code"
}

# Says how to report `$1`, under the title `$2`: a link to a new issue that
# holds it and the kind of system this is, for the Owner to open or not.
report() {
    BODY="$1

System: ${PRETTY_NAME:-unknown} on ${ARCH:-$(uname -m)}
Memory: $((${MEMORY_KB:-0} / 1024)) MB
Installing again over an earlier Orbite: ${UPDATING:-no}

What was shown just before it stopped (please paste it here):
"
    printf 'To report this, open the link below and send the issue it prepares. Nothing is sent unless you open it.\nIt holds only this: what stopped, your system (%s, %s) and its memory.\n\n' "${PRETTY_NAME:-unknown}" "${ARCH:-$(uname -m)}" >&2
    printf 'https://github.com/Zeffut/orbite-agent/issues/new?title=%s&body=%s\n' \
        "$(in_an_address "$2")" "$(in_an_address "$BODY")" >&2
}
trap offer_a_report EXIT

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
ID_LIKE="${ID_LIKE:-}"

# How this system installs packages: by what it says it is made from, or
# else by what is there. None is no obstacle to Orbite itself.
PM=""
case " $ID $ID_LIKE " in
    *" debian "* | *" ubuntu "*) PM=apt-get ;;
    *" fedora "* | *" rhel "* | *" centos "*) PM=dnf ;;
    *" arch "*) PM=pacman ;;
    *" suse "* | *" opensuse "*) PM=zypper ;;
esac
if [ -z "$PM" ] || ! command -v "$PM" >/dev/null 2>&1; then
    PM=""
    for candidate in apt-get dnf pacman zypper; do
        if command -v "$candidate" >/dev/null 2>&1; then
            PM=$candidate
            break
        fi
    done
fi

# Installs the packages named, asking nothing: a question here would read
# the rest of this script as its answer.
# A machine just started is often updating itself: apt then waits for its
# turn rather than failing on the lock.
APT_WAITS="-o DPkg::Lock::Timeout=600"
install_packages() {
    case "$PM" in
        apt-get)
            apt-get $APT_WAITS update -qq </dev/null &&
                DEBIAN_FRONTEND=noninteractive apt-get $APT_WAITS install -y -qq --no-install-recommends "$@" \
                    </dev/null >/dev/null
            ;;
        # dnf leaves in the machine's shared folder the keyring it checks a
        # new source's key with: it is given the installer's own folder,
        # which goes when the installer ends.
        dnf) TMPDIR="${WORK:-/tmp}" dnf install -y -q "$@" </dev/null >/dev/null ;;
        pacman) pacman -Syu --noconfirm --needed "$@" </dev/null >/dev/null ;;
        zypper) zypper --non-interactive install --no-recommends "$@" </dev/null >/dev/null ;;
        *) return 1 ;;
    esac
}

# What the installer brings and the machine did not have is written down:
# what is for the Agent's Computer, for `orbite uninstall` to take away
# again and leave the machine as it was; and the everyday tools (curl,
# sudo), for it to say they were left, as other things may use them by then.
BROUGHT=/usr/local/share/orbite/brought
is_there() {
    case "$PM" in
        apt-get) dpkg -s "$1" 2>/dev/null | grep -q '^Status: install ok installed' ;;
        dnf | zypper) rpm -q "$1" >/dev/null 2>&1 ;;
        pacman) pacman -Q "$1" >/dev/null 2>&1 ;;
        *) return 0 ;;
    esac
}
# `brought package xvfb`, `brought file /etc/...`: written down once.
brought() {
    mkdir -p "$ROOT${BROUGHT%/*}"
    grep -qx "manager $PM" "$ROOT$BROUGHT" 2>/dev/null || echo "manager $PM" >>"$ROOT$BROUGHT"
    grep -qx "$1 $2" "$ROOT$BROUGHT" 2>/dev/null || echo "$1 $2" >>"$ROOT$BROUGHT"
}
# Those of the packages named that the machine does not have yet.
lacking() {
    LACKING=""
    for package in "$@"; do
        is_there "$package" || LACKING="$LACKING $package"
    done
}
# Installs the packages named, and writes down those that are new here.
bring() {
    lacking "$@"
    NEW=$LACKING
    install_packages "$@" || return 1
    for package in $NEW; do
        brought package "$package"
    done
}

# Fetches `$1` into the file `$2`, with whichever of curl and wget is there.
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    else
        wget -q -O "$2" "$1"
    fi
}

# Too old to check a release's signature (it takes OpenSSL 3): said now,
# before anything is fetched or installed.
VERSION_ID="${VERSION_ID:-}"
OLDEST=""
[ "$ID" = debian ] && OLDEST=12
[ "$ID" = ubuntu ] && OLDEST=22.04
if [ -n "$OLDEST" ] && [ -n "$VERSION_ID" ] &&
    [ "$(printf '%s\n%s\n' "$VERSION_ID" "$OLDEST" | sort -V | head -1)" != "$OLDEST" ]; then
    refuse "$PRETTY_NAME is too old: Orbite needs Debian 12 or Ubuntu 22.04, or later."
fi

[ -d "$ROOT/run/systemd/system" ] || refuse "systemd is not running here, and Orbite runs as a systemd service."

[ "$(id -u)" = "0" ] || refuse "it must be run as root: with sudo in front of sh, or from a root shell."

MEMORY_KB=$(sed -n 's/^MemTotal:[^0-9]*\([0-9][0-9]*\).*/\1/p' "$ROOT/proc/meminfo" 2>/dev/null || true)
if [ -n "${MEMORY_KB:-}" ] && [ "$MEMORY_KB" -lt 1900000 ]; then
    say "Note: this machine has little memory ($((MEMORY_KB / 1024)) MB). Orbite will run, but the"
    say "agent's own computer (its desktop and browser) needs about 2 GB to be comfortable."
fi

# --- What the installer itself needs ---

TOOLS=""
MISSING=""
command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 || MISSING="$MISSING curl"
for tool in openssl sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || MISSING="$MISSING $tool"
done
if [ -n "$MISSING" ]; then
    STEP="installing what the installer needs ($(echo $MISSING))"
    say "Installing what the installer needs:$MISSING"
    [ -n "$PM" ] || refuse "this system lacks$MISSING, and this installer does not know how to install packages on it. Install them, then run this again."
    # sha256sum comes with coreutils; certificates with what fetches.
    install_packages ca-certificates curl openssl coreutils
    TOOLS="$TOOLS$MISSING"
fi

# --- The program is downloaded and checked before it is put anywhere ---

WORK=$(mktemp -d)
STEP="downloading Orbite"
FILE="orbite-$ARCH-linux"

say "Downloading Orbite for $ARCH..."
for name in "$FILE" SHA256SUMS SHA256SUMS.sig; do
    fetch "$RELEASE/$name" "$WORK/$name" || refuse "$name could not be downloaded from $RELEASE."
done

STEP="checking what was downloaded"
# The list of checksums is signed; the program must match its checksum.
# Checking the signature takes OpenSSL 3: an older one fails on a sound release.
HAS="$(openssl version 2>/dev/null | cut -d' ' -f2)"
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
    fetch "$RELEASE/VERSION" "$WORK/VERSION" 2>/dev/null &&
    (cd "$WORK" && grep ' VERSION$' SHA256SUMS | sha256sum -c - >/dev/null 2>&1); then
    OFFERED="$(tr -d ' \n' <"$WORK/VERSION")"
    INSTALLED="$("$ROOT$PROGRAM" version 2>/dev/null | cut -d' ' -f2)"
    if [ -n "$INSTALLED" ] && [ "$INSTALLED" != "$OFFERED" ] &&
        [ "$(printf '%s\n%s\n' "$INSTALLED" "$OFFERED" | sort -V | tail -1)" = "$INSTALLED" ]; then
        refuse "this machine runs Orbite $INSTALLED, and what was downloaded is the older $OFFERED. To install it all the same, set ORBITE_ALLOW_OLDER=1."
    fi
fi

# --- Now the machine is changed ---

UPDATING=no
[ -f "$ROOT$SERVICE" ] && UPDATING=yes

STEP="creating the agent's account"
if ! id orbite >/dev/null 2>&1; then
    # What makes accounts is not on the smallest systems.
    if ! command -v useradd >/dev/null 2>&1; then
        case "$PM" in
            apt-get) install_packages passwd ;;
            dnf) install_packages shadow-utils ;;
            *) install_packages shadow ;;
        esac
    fi
    # Where the shell that lets nobody in is differs from one system to the next.
    NOLOGIN=$(command -v nologin 2>/dev/null || echo /usr/sbin/nologin)
    useradd --system --home-dir "$HOME_DIR" --shell "$NOLOGIN" orbite
fi
mkdir -p "$ROOT$HOME_DIR"
# With all that is in it: data kept from an earlier installation was given
# to root when that one was removed.
chown -R orbite: "$ROOT$HOME_DIR"
chmod 700 "$ROOT$HOME_DIR"

STEP="installing the program"
mkdir -p "$ROOT$(dirname "$PROGRAM")"
cp "$WORK/$FILE" "$ROOT$PROGRAM.new"
chmod 755 "$ROOT$PROGRAM.new"
mv "$ROOT$PROGRAM.new" "$ROOT$PROGRAM"

STEP="setting up the service"
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
if [ -f "$ROOT$SERVICE" ] && [ "$(cat "$WORK/service.new")" != "$(cat "$WORK/service.old")" ]; then
    cp "$ROOT$SERVICE" "$ROOT$SERVICE.replaced"
    say "The service file had been changed: it is written anew, and the one before is kept as $SERVICE.replaced. Put changes of your own in a drop-in, which is kept: systemctl edit orbite"
fi
cp "$WORK/orbite.service" "$ROOT$SERVICE"
chmod 644 "$ROOT$SERVICE"

# The Agent keeps its own machine: its account, which is not root, may do
# exactly these as root and nothing else. Whether it does them without
# asking its Owner is the Permission Level's to say, in Orbite.
STEP="letting the agent keep its machine"
# sudo is what lets an account that is not root do a named few things as root.
if ! command -v sudo >/dev/null 2>&1; then
    install_packages sudo 2>/dev/null && TOOLS="$TOOLS sudo" || true
fi
if command -v sudo >/dev/null 2>&1 && [ -d "$ROOT/etc/sudoers.d" ]; then
    cat >"$WORK/sudoers" <<RULES
# Written by Orbite's installer: what the agent's account may do as root.
# What root then runs is looked for where the system keeps its own programs.
Defaults:orbite secure_path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
# Nothing the account puts around a command reaches root.
Defaults:orbite env_reset, !setenv
orbite ALL=(root) NOPASSWD: $PROGRAM update, $PROGRAM update on, $PROGRAM update off
orbite ALL=(root) NOPASSWD: $PROGRAM interface on, $PROGRAM interface off
orbite ALL=(root) NOPASSWD: /usr/bin/systemctl restart orbite, /usr/bin/systemctl reboot, /usr/bin/systemctl poweroff
RULES
    # A rule sudo cannot read would lock sudo for everybody: checked first.
    if ! command -v visudo >/dev/null 2>&1 || visudo -cf "$WORK/sudoers" >/dev/null 2>&1; then
        # In one move, over the one of an earlier install: sudo never
        # reads half a rule.
        cp "$WORK/sudoers" "$ROOT/etc/sudoers.d/.orbite.new"
        chmod 440 "$ROOT/etc/sudoers.d/.orbite.new"
        mv -f "$ROOT/etc/sudoers.d/.orbite.new" "$ROOT/etc/sudoers.d/orbite"
    fi
else
    say "Note: sudo is not on this machine, so the agent cannot update itself or restart the machine"
    say "when you ask it to. Install sudo and run this installer again to let it."
fi

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

# The screen, what moves the pointer and types, and the browser, as each
# system names them; then fonts, each for itself: a page still shows
# without one of them.
computer() {
    case "$PM" in
        apt-get) computer_with_mozillas_firefox ;;
        dnf)
            # Outside Fedora, what moves the pointer is in the extra packages.
            case " $ID " in *" fedora "*) ;; *) bring epel-release 2>/dev/null || true ;; esac
            bring xorg-x11-server-Xvfb xdotool firefox || return 1
            for font in liberation-fonts google-noto-sans-fonts google-noto-sans-cjk-fonts google-noto-color-emoji-fonts; do
                bring "$font" 2>/dev/null || true
            done
            ;;
        pacman)
            bring xorg-server-xvfb xdotool firefox || return 1
            for font in ttf-liberation noto-fonts noto-fonts-cjk noto-fonts-emoji; do
                bring "$font" 2>/dev/null || true
            done
            ;;
        zypper)
            bring xorg-x11-server-Xvfb xdotool MozillaFirefox || return 1
            for font in liberation-fonts google-noto-sans-fonts google-noto-sans-cjk-fonts google-noto-coloremoji-fonts; do
                bring "$font" 2>/dev/null || true
            done
            ;;
        # A system whose packages this installer does not know.
        *) return 2 ;;
    esac
}

computer_with_mozillas_firefox() {
    mkdir -p "$ROOT/etc/apt/keyrings" "$ROOT/etc/apt/sources.list.d" "$ROOT/etc/apt/preferences.d"
    KEYRING=/etc/apt/keyrings/packages.mozilla.org.asc
    for written in "$KEYRING" /etc/apt/sources.list.d/mozilla.list /etc/apt/preferences.d/mozilla; do
        [ -e "$ROOT$written" ] || brought file "$written"
    done
    if [ ! -s "$ROOT$KEYRING" ]; then
        fetch "${ORBITE_MOZILLA_KEY_URL:-https://packages.mozilla.org/apt/repo-signing-key.gpg}" \
            "$ROOT$KEYRING.new" || return 1
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
    apt-get $APT_WAITS update -qq </dev/null || return 1
    set -- xvfb xdotool firefox fonts-liberation fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji
    lacking "$@"
    DEBIAN_FRONTEND=noninteractive apt-get $APT_WAITS install -y -qq --no-install-recommends --allow-downgrades \
        "$@" </dev/null >/dev/null || return 1
    for package in $LACKING; do
        brought package "$package"
    done
}

# Written down now that Orbite is on the machine, not before: an install
# that is refused leaves nothing.
for tool in $TOOLS; do
    brought tool "$tool"
done

if [ "${ORBITE_WITHOUT_COMPUTER:-}" = "1" ]; then
    say "Left out, as asked: the agent's own computer. Without it the agent cannot use sites that need"
    say "signing in or a person's gestures. Run this installer again without ORBITE_WITHOUT_COMPUTER to add it."
else
    say "Setting up the agent's own computer (a screen, Firefox and fonts)..."
    computer && SET_UP=0 || SET_UP=$?
    if [ "$SET_UP" = 2 ]; then
        say "Note: this installer does not know how to install packages on $PRETTY_NAME, so the agent's"
        say "computer was not set up. Orbite works without it. To give it one, install Xvfb, xdotool and"
        say "Firefox (or Chromium) with your system's own tools, then check with: orbite computer check"
    elif [ "$SET_UP" != 0 ]; then
        say "Note: the agent's computer could not be set up. Orbite works without it; run this installer"
        say "again later to add it."
        report "Orbite was installed, but the agent's computer (xvfb, xdotool, Firefox from Mozilla's packages, fonts) could not be set up." "Install: the agent's computer could not be set up"
    fi
fi

STEP="starting Orbite"
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
# Where there is no terminal to ask in, setup is left for later.
if ! (: </dev/tty) 2>/dev/null; then
    say "Orbite is installed. There is no terminal here to set it up in: do it with: sudo orbite setup"
    exit 0
fi
say ""
if ! runuser -u orbite -- env ORBITE_HOME="$HOME_DIR" "$PROGRAM" setup </dev/tty; then
    UNFORESEEN=no
    say "Orbite is installed, but setup was not finished. Go on with: sudo orbite setup"
    exit 1
fi
systemctl start orbite
say "Orbite is running, and will start again by itself after a restart."
