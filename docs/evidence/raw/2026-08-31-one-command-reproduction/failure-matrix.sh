#!/usr/bin/env bash
# Breaks each precondition scripts/reproduce.sh checks, one at a time, and records what it says.
#
# The point is not that the happy path works. It is that every way of getting a wrong number
# silently now stops before a number exists, naming the cause and the command that fixes it.
set -u
cd /srv/soma/audit2/SOMA
export SOMA_REPRODUCE_ROOT=/srv/soma/audit2/scratch
BB=/srv/soma/audit2/scratch/store/busybox-stable-musl-1-512-2048
R="./scripts/reproduce.sh --root /srv/soma/audit2/scratch"

hdr() { printf '\n================ %s\n' "$1"; }
tailout() { tail -6 /tmp/m.out; printf 'exit=%s\n' "$1"; }

hdr "0. control: the store as prepared"
$R --memory-mib 512 --storage-mib 2048 --samples 3 busybox:stable-musl > /tmp/m.out 2>&1; tailout $?

hdr "1. no capture: the snapshot directory is removed"
mv "$BB"/ref-*/snapshot /srv/soma/audit2/snapshot-parked
$R --memory-mib 512 --storage-mib 2048 --samples 3 busybox:stable-musl > /tmp/m.out 2>&1; tailout $?
mv /srv/soma/audit2/snapshot-parked "$(echo "$BB"/ref-*)/snapshot"

hdr "2. wrong shape: a store captured at 512 MiB asked for 1024"
$R --store "$BB" --memory-mib 1024 --storage-mib 2048 --samples 3 busybox:stable-musl > /tmp/m.out 2>&1; tailout $?

hdr "3. dangling kernel link"
mv kernel/out /tmp/kout.link 2>/dev/null; ln -sfn /srv/soma/no-such-kernel kernel/out
$R --memory-mib 512 --storage-mib 2048 --samples 3 busybox:stable-musl > /tmp/m.out 2>&1; tailout $?
rm -f kernel/out; mv /tmp/kout.link kernel/out

hdr "4a. stale store: prepared under an older wire contract, no stamp"
$R --store /srv/soma/sweep/node-1024-10240 --memory-mib 1024 --storage-mib 10240 \
   --expect v22 node:22 -- /usr/local/bin/node --version > /tmp/m.out 2>&1; tailout $?

hdr "4b. stale store: a stamp naming a wire contract this checkout does not have"
cp "$BB/.soma-reproduce-stamp" /tmp/stamp.bak
sed -i 's/^contract=.*/contract=0000badc0ntract/' "$BB/.soma-reproduce-stamp"
$R --store "$BB" --memory-mib 512 --storage-mib 2048 --samples 3 busybox:stable-musl > /tmp/m.out 2>&1; tailout $?
cp /tmp/stamp.bak "$BB/.soma-reproduce-stamp"

hdr "5. a command after -- with no --expect"
$R --memory-mib 512 --storage-mib 2048 busybox:stable-musl -- /bin/busybox --help > /tmp/m.out 2>&1; tailout $?

hdr "6. cargo missing from a non-interactive PATH, and ~/.cargo/env hidden"
mv "$HOME/.cargo/env" "$HOME/.cargo/env.parked"
env -i HOME="$HOME" PATH=/usr/local/bin:/usr/bin:/bin bash -c \
  "cd /srv/soma/audit2/SOMA && SOMA_REPRODUCE_ROOT=/srv/soma/audit2/scratch ./scripts/reproduce.sh --memory-mib 512 --storage-mib 2048 busybox:stable-musl" > /tmp/m.out 2>&1; tailout $?
mv "$HOME/.cargo/env.parked" "$HOME/.cargo/env"

hdr "7. control again: nothing above left damage"
$R --memory-mib 512 --storage-mib 2048 --samples 3 busybox:stable-musl > /tmp/m.out 2>&1; tailout $?
