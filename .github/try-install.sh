#!/bin/sh
# Installs Orbite on a fresh machine from the release in the folder given,
# and checks what an Owner counts on: the service runs, comes back after a
# crash, starts with the machine, and installing again changes nothing else.
set -eu
# When something below fails, what the service was doing is shown.
trap 'code=$?; [ $code -eq 0 ] || { systemctl status orbite --no-pager -l || true; journalctl -u orbite -n 30 --no-pager || true; }; exit $code' EXIT
RELEASE="$1"
case "$(uname -m)" in aarch64 | arm64) FILE=orbite-aarch64-linux ;; *) FILE=orbite-x86_64-linux ;; esac
export ORBITE_RELEASE_URL="file://$RELEASE"
ORBITE_SIGNING_KEY="$(cat "$RELEASE/try.pub")"
export ORBITE_SIGNING_KEY
export ORBITE_SKIP_SETUP=1

sh "$RELEASE/install.sh"

/usr/local/bin/orbite providers >/dev/null
id orbite >/dev/null
[ "$(stat -c '%U %a' /var/lib/orbite)" = "orbite 700" ]
[ "$(systemctl is-enabled orbite)" = "enabled" ]

# Enough of a setup for the service to stay up: a model it will never reach,
# and a stand-in for Telegram that never has anything to pass on.
python3 "$RELEASE/fake-telegram.py" 28932 &
TELEGRAM=$!
as_orbite() { runuser -u orbite -- env ORBITE_HOME=/var/lib/orbite "$@"; }
printf '%s' '{"provider":{"name":"custom","model":"none","address":"http://127.0.0.1:9"},"telegram_owner":1,"telegram_address":"http://127.0.0.1:28932"}' |
    as_orbite tee /var/lib/orbite/config.json >/dev/null
printf '%s\n' "1234567890:ForTheInstallTestOnlyNotARealToken" |
    as_orbite /usr/local/bin/orbite secret set ORBITE_TELEGRAM_TOKEN >/dev/null

systemctl start orbite
sleep 3
[ "$(systemctl is-active orbite)" = "active" ]
FIRST=$(systemctl show -p MainPID --value orbite)

# It comes back by itself after a crash.
kill -9 "$FIRST"
sleep 8
[ "$(systemctl is-active orbite)" = "active" ]
SECOND=$(systemctl show -p MainPID --value orbite)
[ "$FIRST" != "$SECOND" ]

# Installing again keeps the Owner's data and leaves the service running.
# This time as the guide says to: the script read by sh from its input.
sh < "$RELEASE/install.sh"
grep -q '"telegram_owner":1' /var/lib/orbite/config.json
sleep 3
[ "$(systemctl is-active orbite)" = "active" ]

# Asked as root with nothing said of where, a command works on the installed
# Orbite and leaves what it writes to the Agent's account.
printf '%s\n' "a-value-for-the-install-trial" | /usr/local/bin/orbite secret set TRIAL_KEY >/dev/null
/usr/local/bin/orbite settings quiet 22:00-07:00 >/dev/null
[ -z "$(find /var/lib/orbite ! -user orbite)" ]
as_orbite /usr/local/bin/orbite secret list | grep -q TRIAL_KEY
as_orbite /usr/local/bin/orbite settings | grep -q "quiet = 22:00-07:00"

if [ "${ORBITE_WITHOUT_COMPUTER:-}" = 1 ]; then
    # Asked to leave the Computer out: nothing of it was brought.
    if command -v firefox >/dev/null || command -v Xvfb >/dev/null; then
        echo "the Computer was installed though it was left out" >&2
        exit 1
    fi
else
    # The Agent's Computer was set up, and works under the Agent's own account.
    command -v Xvfb >/dev/null && command -v xdotool >/dev/null && command -v firefox >/dev/null
    /usr/local/bin/orbite computer check
    [ "$(stat -c '%U' /var/lib/orbite/computer/check.png)" = "orbite" ]
fi

# It keeps itself up to date.
[ "$(systemctl is-enabled orbite-update.timer)" = "enabled" ]
[ "$(systemctl is-active orbite-update.timer)" = "active" ]
(cd "$RELEASE" && exec python3 -m http.server 28931 --bind 127.0.0.1 >/dev/null 2>&1) &
SERVER=$!
sleep 1
update_from() {
    ORBITE_HOME=/var/lib/orbite ORBITE_RELEASE_URL="http://127.0.0.1:28931/$1" /usr/local/bin/orbite update
}
# A release that does not start is undone, and the one before runs again.
if update_from broken; then
    echo "an update that does not start was kept" >&2
    exit 1
fi
sleep 5
[ "$(systemctl is-active orbite)" = "active" ]
/usr/local/bin/orbite version >/dev/null
# A release that works is installed, and the service runs it.
BEFORE=$(systemctl show -p MainPID --value orbite)
update_from next | grep -q "Updated to 99.0.0"
[ "$(systemctl is-active orbite)" = "active" ]
[ "$BEFORE" != "$(systemctl show -p MainPID --value orbite)" ]
[ -f /usr/local/bin/orbite.previous ]
# The Agent, which is not root, can take what was left for the Owner.
[ "$(stat -c '%U' /var/lib/orbite/notices)" = "orbite" ]
kill "$SERVER"

# Moving to another Host: everything is exported, Orbite is removed without
# a trace, installed again, and the export put back gives the same Agent.
ORBITE_HOME=/var/lib/orbite /usr/local/bin/orbite export /root/orbite-export.tar.gz
[ "$(stat -c '%a' /root/orbite-export.tar.gz)" = "600" ]
cp /usr/local/bin/orbite /root/orbite-to-uninstall
/usr/local/bin/orbite uninstall --yes
[ ! -e /usr/local/bin/orbite ] && [ ! -e /var/lib/orbite ] && [ ! -e /etc/systemd/system/orbite.service ]
if id orbite >/dev/null 2>&1; then
    echo "the account was left behind" >&2
    exit 1
fi
if systemctl is-active --quiet orbite; then
    echo "the service was left running" >&2
    exit 1
fi
# As from a script with no terminal to ask in: installed, setup left for later.
env -u ORBITE_SKIP_SETUP setsid sh "$RELEASE/install.sh" </dev/null | tee /root/install.out
grep -q "no terminal here" /root/install.out
ORBITE_HOME=/var/lib/orbite /usr/local/bin/orbite restore /root/orbite-export.tar.gz
grep -q '"telegram_owner":1' /var/lib/orbite/config.json
[ -z "$(find /var/lib/orbite ! -user orbite)" ]
as_orbite /usr/local/bin/orbite secret list | grep -q ORBITE_TELEGRAM_TOKEN
systemctl restart orbite
sleep 3
[ "$(systemctl is-active orbite)" = "active" ]
kill "$TELEGRAM"

# What was tampered with is refused.
echo tampered >> "$RELEASE/$FILE"
if sh "$RELEASE/install.sh" 2>/dev/null; then
    echo "a tampered program was installed" >&2
    exit 1
fi
echo "Installed, restarted after a crash, updated in place and by itself, undid a bad update, moved by export and restore, and refused a tampered program."
