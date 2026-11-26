#!/bin/sh
# Starts a real virtual machine from the image a system publishes, with
# nothing added to it, installs Orbite there as an Owner would, tries the
# service, then restarts the machine and checks Orbite came back by itself.
#   try-on-a-machine.sh IMAGE-ADDRESS RELEASE-FOLDER
set -eu
IMAGE="$1"
RELEASE="$2"
WORK=$(mktemp -d)
trap 'code=$?; [ $code -eq 0 ] || { echo "--- the end of what the machine wrote while starting ---"; sudo tail -n 60 "$WORK/console.log" 2>/dev/null || true; }; sudo pkill -F "$WORK/pid" 2>/dev/null || true; exit $code' EXIT

echo "The machine: $IMAGE"
curl -fsSL --retry 3 -o "$WORK/disk.img" "$IMAGE"
qemu-img resize "$WORK/disk.img" 24G >/dev/null

# The only thing given to the machine: an account to reach it by.
ssh-keygen -q -t ed25519 -N "" -f "$WORK/key"
cat > "$WORK/user-data" <<DATA
#cloud-config
users:
  - name: owner
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    lock_passwd: true
    shell: /bin/sh
    ssh_authorized_keys:
      - $(cat "$WORK/key.pub")
DATA
printf 'instance-id: orbite-trial\nlocal-hostname: fresh\n' > "$WORK/meta-data"
cloud-localds "$WORK/seed.img" "$WORK/user-data" "$WORK/meta-data"

sudo qemu-system-x86_64 -enable-kvm -cpu host -m 4096 -smp 2 \
    -drive "file=$WORK/disk.img,if=virtio" \
    -drive "file=$WORK/seed.img,if=virtio,format=raw" \
    -netdev user,id=net,hostfwd=tcp:127.0.0.1:2222-:22 -device virtio-net-pci,netdev=net \
    -display none -serial "file:$WORK/console.log" -pidfile "$WORK/pid" -daemonize

there() {
    ssh -q -i "$WORK/key" -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=5 -o ServerAliveInterval=15 owner@127.0.0.1 "$@"
}
reached() {
    tries=0
    until there true 2>/dev/null; do
        tries=$((tries + 1))
        [ "$tries" -lt 90 ] || { echo "the machine was not reached" >&2; return 1; }
        sleep 5
    done
}

reached
there 'cat /etc/os-release | grep PRETTY_NAME; uname -r'
tar -C "$(dirname "$RELEASE")" -c "$(basename "$RELEASE")" | there "sudo tar -x -C / --no-same-owner"
there "sudo sh /$(basename "$RELEASE")/try-install.sh /$(basename "$RELEASE")"

# The machine is restarted: Orbite starts with it, nobody doing anything.
there "sudo systemctl reboot" || true
sleep 20
reached
tries=0
until [ "$(there 'systemctl is-active orbite' || true)" = "active" ]; do
    tries=$((tries + 1))
    [ "$tries" -lt 12 ] || { there 'systemctl status orbite --no-pager -l; sudo journalctl -u orbite -n 30 --no-pager' || true; echo "Orbite did not come back with the machine" >&2; exit 1; }
    sleep 5
done
[ "$(there 'systemctl is-active orbite-update.timer')" = "active" ]
echo "Installed on the machine as published, and back by itself after the machine restarted."

# Where asked, the same machine then holds its agent in a container: Docker
# is brought by the installer, as on any machine that has none.
if [ "${TRY_SERVER:-}" = 1 ]; then
    there "sudo sh /$(basename "$RELEASE")/try-server.sh /$(basename "$RELEASE")"
fi
