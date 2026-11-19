#!/bin/sh
# Installs Orbite on a fresh machine from the release in the folder given,
# and checks what an Owner counts on: the service runs, comes back after a
# crash, starts with the machine, and installing again changes nothing else.
set -eu
RELEASE="$1"
export ORBITE_RELEASE_URL="file://$RELEASE"
ORBITE_SIGNING_KEY="$(cat "$RELEASE/try.pub")"
export ORBITE_SIGNING_KEY
export ORBITE_SKIP_SETUP=1

sh "$RELEASE/install.sh"

/usr/local/bin/orbite providers >/dev/null
id orbite >/dev/null
[ "$(stat -c '%U %a' /var/lib/orbite)" = "orbite 700" ]
[ "$(systemctl is-enabled orbite)" = "enabled" ]

# Enough of a setup for the service to stay up: a bot it will never reach.
as_orbite() { runuser -u orbite -- env ORBITE_HOME=/var/lib/orbite "$@"; }
printf '%s' '{"provider":{"name":"custom","model":"none","address":"http://127.0.0.1:9"},"telegram_owner":1}' |
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
sh "$RELEASE/install.sh"
grep -q '"telegram_owner":1' /var/lib/orbite/config.json
sleep 3
[ "$(systemctl is-active orbite)" = "active" ]

# What was tampered with is refused.
echo tampered >> "$RELEASE/orbite-x86_64-linux"
if sh "$RELEASE/install.sh" 2>/dev/null; then
    echo "a tampered program was installed" >&2
    exit 1
fi
echo "Installed, restarted after a crash, updated in place, and refused a tampered program."
