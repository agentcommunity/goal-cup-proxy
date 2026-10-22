#!/bin/sh
# T25: measure what a run does today on a literal ENOSPC (WP-C, invariant I3). Measurement only.
#
#   sudo sh scripts/t25-disk-full.sh [row ...]
#
# Rows (default: uploads-working chat-continue local-stop local-stop-zero):
#   uploads-working   S1, S3, S4: upload stand-in up; fill under the 256 MiB floor, then to zero,
#                     then free; then end the harness, status, verify, continue, seal
#   chat-continue     S3, S4 on a chat-mode run (fake TUI), then kill the agent and continue
#   local-stop        S2 without a server (uploads count as failing at once): the stop verdict
#   local-stop-zero   the same, filled to zero: the stop races a literal ENOSPC
#   server-down       S2 with the stand-in stopped and a trickling harness: waits the 30-minute
#                     failing-upload window (about 35 minutes)
#
# Linux and root only: each row gets a fresh ext4 image (mkfs -m 0, so root's reserved blocks do
# not hide the fill), loop-mounted, with TMPDIR pointing into it, so the configuration directory,
# the daemon's artifacts, the run directory and the reserve are all on it. Logs, the workload's
# report and the upload stand-in's store go to $T25_WORK/out/<row>, off the volume. The image is
# unmounted and deleted after each row and on any exit.
#
# Environment: T25_WORK (default /var/tmp/t25; holds the image, the mount and out/),
# T25_IMAGE_GIB (default 5: the shipped 3 GiB reserve plus the 1 GiB start margin must fit).
set -eu

if [ "$(uname -s)" != Linux ] || [ "$(id -u)" != 0 ]; then
  echo "t25-disk-full: Linux and root only (it loop-mounts an image)" >&2
  exit 64
fi

here=$(cd "$(dirname "$0")/.." && pwd)
work=${T25_WORK:-/var/tmp/t25}
gib=${T25_IMAGE_GIB:-5}
image="$work/volume.img"
mnt="$work/mnt"
out="$work/out"
rows=${*:-uploads-working chat-continue local-stop local-stop-zero}

hosts_mark="# t25-disk-full"
unhosts() {
  if grep -q "$hosts_mark" /etc/hosts 2>/dev/null; then
    grep -v "$hosts_mark" /etc/hosts >"$work/hosts.keep" && cat "$work/hosts.keep" >/etc/hosts
    rm -f "$work/hosts.keep"
  fi
}
cleanup() {
  if mountpoint -q "$mnt" 2>/dev/null; then
    fuser -km "$mnt" >/dev/null 2>&1 || true
    sleep 1
    umount "$mnt" 2>/dev/null || umount -l "$mnt" 2>/dev/null || true
  fi
  rm -f "$image"
}
# On a signal, exit (the EXIT trap cleans up); never go on with the next row.
trap 'cleanup; unhosts' EXIT
trap 'exit 130' INT TERM HUP

mkdir -p "$work" "$mnt" "$out"
# The controlled endpoint's names (RFC 6761 `.localhost`) do not resolve on every resolver;
# glibc without systemd-resolved needs them in /etc/hosts. Removed again on exit.
if ! getent hosts api.e00.localhost >/dev/null 2>&1; then
  printf '127.0.0.1 api.e00.localhost auth.e00.localhost %s\n' "$hosts_mark" >>/etc/hosts
fi
cd "$here"
# Build with the real TMPDIR: rustc's own temporary files must not land on the volume.
cargo test -p goalcup-cli --offline --features test-support --test full --no-run
cargo build --offline -p goalcup-daemon -p goalcup-verifier

for row in $rows; do
  case $row in
    uploads-working) test=t25_enospc_with_uploads_working; extra= ;;
    chat-continue) test=t25_enospc_chat_run_then_continue; extra= ;;
    local-stop) test=t25_stop_verdict_local_run; extra=T25_LOCAL_TARGET_MIB=128 ;;
    local-stop-zero) test=t25_stop_verdict_local_run; extra=T25_LOCAL_TARGET_MIB=0 ;;
    server-down) test=t25_stop_verdict_server_down; extra= ;;
    *) echo "t25-disk-full: unknown row $row" >&2; exit 64 ;;
  esac
  cleanup
  truncate -s "${gib}G" "$image"
  mkfs.ext4 -q -F -m 0 "$image"
  mount -o loop "$image" "$mnt"
  mkdir -p "$mnt/tmp"
  echo "t25-disk-full: $row on $(df -h --output=size,avail "$mnt" | tail -1)"
  began=$(date +%s)
  set +e
  env TMPDIR="$mnt/tmp" T25_OUT="$out" $extra \
    cargo test -p goalcup-cli --offline --features test-support --test full \
      "t25_disk_full::$test" -- --ignored --exact --nocapture --test-threads=1 \
      >"$out/$row.cargo.log" 2>&1
  code=$?
  set -e
  echo "t25-disk-full: $row exit $code after $(( $(date +%s) - began )) s; evidence in $out"
  cleanup
done
