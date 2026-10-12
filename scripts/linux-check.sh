#!/bin/sh
# The Linux half of the gate, run in Docker (from a Mac, or any host that runs Docker).
# Needs: Docker that allows privileged containers (ext4 and XFS volumes are loop-mounted inside),
# and `cargo fetch --locked` run on the host first, because the container builds with --offline
# from the host's cargo registry. The first run downloads the rust:1.99 image, Debian packages and
# the cargo-nextest binary; the build cache is the volume goalcup-linux-check-target.
# Usage: sh scripts/linux-check.sh      Last line: `linux-check: PASS` or `linux-check: FAIL <step>`.
set -u
cd "$(dirname "$0")/.."
img=goalcup-linux-check
name=goalcup-linux-check-$$
registry=${CARGO_HOME:-$HOME/.cargo}/registry

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

# --init: something must reap orphans, as it does on a real host or WSL2.
trap 'docker rm -f "$name" >/dev/null 2>&1' EXIT INT TERM
docker run -d --name "$name" --privileged --init --cpus=6 -v "$PWD":/src \
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
    docker exec -u dev -e HOME=/home/dev -e TMPDIR=/mnt/ext4/tmp "$name" sh -c "$2" ||
        { echo "linux-check: FAIL $1"; exit 1; }
}
step "fast gate" 'sh scripts/gate.sh fast'
step "reservation" '
    GOALCUP_TRACECAP_TEST_ROOTS=/mnt/ext4/tmp:/mnt/xfs/tmp GOALCUP_TRACECAP_REQUIRE_QUALIFIED=1 \
        cargo test --offline -p goalcup-platform --lib linux_reservation &&
    for t in /tmp /mnt/ext4/tmp /mnt/xfs/tmp; do
        TMPDIR=$t cargo test --offline -p goalcup-protocol --test capture_reserved_storage || exit 1
    done'
step "cli end-to-end" 'cargo nextest run --offline -p goalcup-cli --test full --test m2_cli_lifecycle'
step "daemon full" 'cargo nextest run --offline -p goalcup-daemon --test full'
echo "linux-check: PASS"
