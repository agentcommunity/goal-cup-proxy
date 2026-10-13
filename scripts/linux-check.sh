#!/bin/sh
# The Linux half of the gate, run in Docker. Measured on Docker Desktop on an arm64 Mac only;
# x86_64 hosts and native Linux Docker are untested.
# Needs: Docker that allows privileged containers (ext4 and XFS volumes are loop-mounted inside),
# and `cargo fetch --locked` run on the host first, because the container builds with --offline
# from the host's cargo registry. The first run downloads the rust:1.99 image, Debian packages and
# the cargo-nextest binary; the build cache is the volume goalcup-linux-check-target.
# The script takes no lock itself: do not run it alongside other heavy test runs.
# Usage: sh scripts/linux-check.sh      (LINUX_CHECK_HARNESSES=1 adds the harness smoke)
# Last line: `linux-check: PASS` or `linux-check: FAIL <step>`.
set -u
# CARGO_HOME may be relative to the caller's directory; docker -v needs an absolute path.
cargo_home=${CARGO_HOME:-$HOME/.cargo}
case $cargo_home in /*) ;; *) cargo_home=$(pwd)/$cargo_home ;; esac
cd "$(dirname "$0")/.." || exit 1
img=goalcup-linux-check
name=goalcup-linux-check-$$
# Labels the containers of this checkout only, so the sweep never removes another checkout's.
label=goalcup-linux-check=$(printf %s "$PWD" | cksum | cut -d' ' -f1)
registry=$cargo_home/registry

[ -d "$registry" ] || { echo "linux-check: FAIL registry (run cargo fetch --locked first)"; exit 1; }
docker build -q -t "$img" - >/dev/null <<'DOCKERFILE' || { echo "linux-check: FAIL image"; exit 1; }
FROM rust:1.99
RUN case "$(uname -m)" in aarch64) n=linux-arm ;; *) n=linux ;; esac \
 && rustup component add clippy rustfmt \
 && apt-get update && apt-get install -y --no-install-recommends \
    e2fsprogs xfsprogs python3 netcat-openbsd libnss-myhostname \
 && rm -rf /var/lib/apt/lists/* \
 && sed -i 's/^hosts:.*/hosts:          files myhostname dns/' /etc/nsswitch.conf \
 && curl -LsSf "https://get.nexte.st/latest/$n" | tar zxf - -C /usr/local/cargo/bin \
 && useradd -m -u 1000 dev && cargo nextest --version
ENV RUSTUP_TOOLCHAIN=1.99.0 CARGO_TARGET_DIR=/target CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
DOCKERFILE

# A hard-killed earlier run (SIGKILL skips the trap) leaves its container; sweep it first.
old=$(docker ps -aq --filter "label=$label")
[ -z "$old" ] || docker rm -f $old >/dev/null 2>&1
trap 'docker rm -f "$name" >/dev/null 2>&1' EXIT
trap 'exit 130' INT HUP TERM
# --cpus is capped at what the Docker host has; a higher value is refused.
cpus=$(docker info --format '{{.NCPU}}' 2>/dev/null)
case $cpus in '' | *[!0-9]* | 0) cpus=6 ;; esac
[ "$cpus" -le 6 ] || cpus=6
# --init: something must reap orphans, as it does on a real host or WSL2.
docker run -d --name "$name" --label "$label" --privileged --init --cpus="$cpus" -v "$PWD":/src \
    -v "$registry":/usr/local/cargo/registry -v goalcup-linux-check-target:/target \
    -w /src "$img" sleep infinity >/dev/null || { echo "linux-check: FAIL container"; exit 1; }
docker exec "$name" sh -c 'set -e; chown dev /target; mkdir -p /mnt/ext4 /mnt/xfs
    truncate -s 4G /ext4.img; mkfs.ext4 -q -F /ext4.img; mount -o loop /ext4.img /mnt/ext4
    truncate -s 2G /xfs.img; mkfs.xfs -q -f /xfs.img; mount -o loop /xfs.img /mnt/xfs
    mkdir -p /mnt/ext4/tmp /mnt/xfs/tmp; chmod 1777 /mnt/ext4/tmp /mnt/xfs/tmp' ||
    { echo "linux-check: FAIL volumes"; exit 1; }

# step NAME COMMAND: as dev, with TMPDIR on ext4 (/tmp is overlayfs, which captures refuse).
step() {
    echo "linux-check: $1"
    # In the background plus `wait`, so a signal runs the trap now, not after the step ends.
    docker exec -u dev -e HOME=/home/dev -e TMPDIR=/mnt/ext4/tmp "$name" sh -c "$2" &
    wait $! || { echo "linux-check: FAIL $1"; exit 1; }
}
step "registry" 'cargo metadata --offline --locked --format-version 1 >/dev/null'
step "fast gate" 'sh scripts/gate.sh fast'
step "reservation" '
    GOALCUP_TRACECAP_TEST_ROOTS=/mnt/ext4/tmp:/mnt/xfs/tmp GOALCUP_TRACECAP_REQUIRE_QUALIFIED=1 \
        cargo test --offline -p goalcup-platform --lib linux_reservation &&
    for t in /tmp /mnt/ext4/tmp /mnt/xfs/tmp; do
        TMPDIR=$t cargo test --offline -p goalcup-protocol --test capture_reserved_storage || exit 1
    done'
step "cli end-to-end" 'cargo nextest run --offline -p goalcup-cli --test full --test m2_cli_lifecycle'
# The `#[ignore]`d end-to-end suites, selected as `gate.sh full` selects them.
step "ignored end-to-end" 'cargo nextest run --offline -p goalcup-cli --test full --run-ignored ignored-only \
    -E "test(/^(m2_synthetic_vertical|capture_failure_campaign)::/)"'
step "daemon full" 'cargo nextest run --offline -p goalcup-daemon --test full'
# Optional, needs network: the measured harness builds for this machine's architecture, fetched
# from npm and launched with no login to the auth step (scripts/linux-harness-smoke.sh). The
# rebuild puts the shipped binaries back in /target/debug after the test builds.
if [ "${LINUX_CHECK_HARNESSES:-0}" = 1 ]; then
    step "harness smoke" 'cargo build --offline --locked --workspace &&
        GOALCUP_CONNECTORS_DIR=/src/connectors PATH=/target/debug:$PATH sh scripts/linux-harness-smoke.sh'
fi
echo "linux-check: PASS"
