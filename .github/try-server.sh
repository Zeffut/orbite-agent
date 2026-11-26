#!/bin/sh
# Installs Orbite on a machine that has Docker, from the release in the
# folder given, and checks what an Owner counts on there: the agent they
# set up runs in a container of its own with everything it kept, the
# machine holds the server and nothing of the agent, and removing Orbite
# leaves no container behind.
set -eu
RELEASE=$1
export ORBITE_RELEASE_URL="file://$RELEASE"
ORBITE_SIGNING_KEY="$(cat "$RELEASE/try.pub")"
export ORBITE_SIGNING_KEY
export ORBITE_SKIP_SETUP=1
# Shown when something below fails.
trap 'code=$?; [ "$code" = 0 ] || { journalctl -u orbite -n 60 --no-pager; docker ps -a; }' EXIT

# What a trial before this one left.
[ ! -x /usr/local/bin/orbite ] || /usr/local/bin/orbite uninstall --yes || true

sh "$RELEASE/install.sh"
# Docker is there: agents will run in containers, and no computer was
# brought for one on the machine itself.
test -e /usr/local/share/orbite/containers
if grep -q 'package firefox' /usr/local/share/orbite/brought 2>/dev/null; then
    echo "not as an Owner counts on: grep -q 'package firefox' /usr/local/share/orbite/brought 2>" >&2
    exit 1
fi

# Enough of a setup for an agent to be there: a model it will never reach,
# its page, something remembered and a secret of its own.
as_orbite() { runuser -u orbite -- env ORBITE_HOME=/var/lib/orbite "$@"; }
cat >/var/lib/orbite/config.json <<JSON
{"language":"en","provider":{"name":"openai","model":"a-model"},"agent_name":"Echo","interface":"127.0.0.1:8377"}
JSON
mkdir -p /var/lib/orbite/memory
echo "likes tea" >/var/lib/orbite/memory/a-note.md
chown -R orbite: /var/lib/orbite
printf 'a-key-that-opens-nothing\n' | as_orbite /usr/local/bin/orbite secret set ORBITE_PROVIDER_KEY >/dev/null
printf 'its-own\n' | as_orbite /usr/local/bin/orbite secret set WEBMAIL_PASSWORD >/dev/null

# The installer run again, as it is after a setup: the agent moves into a
# container of its own.
sh "$RELEASE/install.sh"
test -f /var/lib/orbite/agents.json
systemctl is-active --quiet orbite
NAMED=$(docker ps --format '{{.Names}}' | grep '^orbite-agent-')
[ "$(echo "$NAMED" | wc -l)" = 1 ]
# Everything of its own went with it, and nothing of it stayed.
docker exec "$NAMED" cat /var/lib/orbite/memory/a-note.md | grep -q 'likes tea'
docker exec "$NAMED" orbite secret list | grep -q WEBMAIL_PASSWORD
if docker exec "$NAMED" orbite secret list | grep -q ORBITE_PROVIDER_KEY; then
    echo "not as an Owner counts on: docker exec \"\$NAMED\" orbite secret list | grep -q ORBITE_" >&2
    exit 1
fi
[ ! -e /var/lib/orbite/memory ]
as_orbite /usr/local/bin/orbite secret list | grep -q ORBITE_PROVIDER_KEY
if as_orbite /usr/local/bin/orbite secret list | grep -q WEBMAIL_PASSWORD; then
    echo "not as an Owner counts on: as_orbite /usr/local/bin/orbite secret list | grep -q WEBMAI" >&2
    exit 1
fi
# Nothing in the folder became root's.
[ -z "$(find /var/lib/orbite ! -user orbite)" ]
# The server answers: its page to anybody, its agents to devices let in only.
# (Asked with whichever of curl and wget the machine has.)
answered() {
    if command -v curl >/dev/null 2>&1; then
        curl -s -o /dev/null -w '%{http_code}' "$1"
    else
        wget -q -S -O /dev/null "$1" 2>&1 | awk '/HTTP\//{code=$2} END{print code}'
    fi
}
[ "$(answered http://127.0.0.1:8377/)" = 200 ]
[ "$(answered http://127.0.0.1:8377/api/agents)" = 401 ]
# Asked a second time, there is nothing to do, and nothing is undone.
if /usr/local/bin/orbite server adopt; then
    echo "not as an Owner counts on: /usr/local/bin/orbite server adopt" >&2
    exit 1
fi
test -f /var/lib/orbite/agents.json
# The service started again finds its agent as it was.
systemctl restart orbite
sleep 8
systemctl is-active --quiet orbite
docker ps --format '{{.Names}} {{.Status}}' | grep -q "^$NAMED Up"

# Everything is saved to one file, agents included; removed, Orbite leaves
# no container, no volume and no network of an agent.
/usr/local/bin/orbite export /root/orbite-server.orbite
tar -tzf /root/orbite-server.orbite | grep -q '^agents/.*\.orbite$'
/usr/local/bin/orbite uninstall --yes
if docker ps -a --format '{{.Names}}' | grep -q '^orbite-agent-'; then
    echo "not as an Owner counts on: docker ps -a --format '{{.Names}}' | grep -q '^orbite-agent-" >&2
    exit 1
fi
if docker volume ls -q | grep -q '^orbite-agent-'; then
    echo "not as an Owner counts on: docker volume ls -q | grep -q '^orbite-agent-'" >&2
    exit 1
fi
if docker network ls --format '{{.Name}}' | grep -q '^orbite-agent-'; then
    echo "not as an Owner counts on: docker network ls --format '{{.Name}}' | grep -q '^orbite-ag" >&2
    exit 1
fi
# Nor what it wrote for the machine to serve agents.
[ ! -e /usr/local/share/orbite ]
[ ! -e /etc/systemd/system/orbite.service.d/docker.conf ]
if docker image ls --format '{{.Repository}}' | grep -q '^orbite-agent$'; then
    echo "an agent's image was left behind" >&2
    exit 1
fi
rm -f /root/orbite-server.orbite /var/lib/orbite-before-server-*.orbite
echo "A server that holds its agent in a container: as an Owner counts on."
