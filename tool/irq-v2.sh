#!/usr/bin/env bash
# IRQ v2 (WIP) build/run helper. Reproduces the lwz setup (container kmiri-3871-analysis-20261109)
# on any Docker host: same toolchain image, KMiri checkout mounted at /KMiri.
#
#   tool/irq-v2.sh build                build miri, cargo-miri, cargo-osdk and the x86_64-unknown-none sysroot
#   tool/irq-v2.sh run TEST [CONFIG]    run tests/TEST with tests/irq-v2-configs/CONFIG
#                                       (default asterinas-irq-tick-10k.toml); prints exit code and the
#                                       SHA256 of the "KMiri IRQ:" trace
#   tool/irq-v2.sh shell                interactive shell with the same environment
#
# The *-m1648 tests need a mutant tree: git -C asterinas worktree add ../asterinas-m1648 irq-v2-m1648
# and a cargo-osdk built from it; `run` does not handle that case.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
IMG=${IRQ_V2_IMG:-zjpzjp/blueos:nightly-2026-07-21_v3}   # rustc 1.99.0-dev (83b1b18c4 2026-09-13)
CTR=${IRQ_V2_CTR:-kmiri-irq-v2}
VOL=${IRQ_V2_VOL:-kmiri-irq-v2-work}                       # /work: tools, sysroot, cargo target dirs
LOGS=${IRQ_V2_LOGS:-$HOME/logs/irq-v2}

if ! docker container inspect "$CTR" >/dev/null 2>&1; then
  docker run -d --name "$CTR" --network host -v "$ROOT":/KMiri -v "$VOL":/work "$IMG" sleep infinity >/dev/null
fi
[ "$(docker container inspect -f '{{.State.Running}}' "$CTR")" = true ] || docker start "$CTR" >/dev/null

ENV='
export LD_LIBRARY_PATH=/opt/sysroot/usr/local/lib:/opt/sysroot/usr/local/lib/rustlib/x86_64-unknown-linux-gnu/lib
export MIRI=/KMiri/kmiri/target/debug/miri
export MIRI_SYSROOT=/work/sysroot-kernel
export VDSO_LIBRARY_DIR=/KMiri/env/linux_vdso
export OSDK_LOCAL_DEV=1 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=${JOBS:-8}
export CARGO_PROFILE_DEV_OPT_LEVEL=2   # lwz built miri this way; an unoptimized miri is ~10x slower
export PATH=/work/tools/bin:/KMiri/kmiri/target/debug:/KMiri/kmiri/cargo-miri/target/debug:/opt/sysroot/usr/local/bin:/usr/local/bin:/usr/bin:/bin
unset RUSTUP_TOOLCHAIN RUSTC RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
'
in_ctr() { docker exec -i -e JOBS="${JOBS:-8}" "$CTR" bash -c "$ENV
$1"; }

case "${1:-}" in
  build)
    in_ctr 'set -e
      cd /KMiri/kmiri && cargo build --locked --bin miri
      cd /KMiri/kmiri/cargo-miri && cargo build --locked
      cd /KMiri/asterinas && cargo install --locked --path osdk --root /work/tools
      cargo miri setup --target x86_64-unknown-none
      sha256sum /KMiri/kmiri/target/debug/miri' ;;
  run)
    t=${2:?test name}; cfg=${3:-asterinas-irq-tick-10k.toml}
    mkdir -p "$LOGS"; log="$LOGS/$t-$(date +%Y%m%d-%H%M%S).log"
    set +e
    in_ctr "cd /KMiri/tests/$t && CARGO_TARGET_DIR=/work/integration-target \
      MIRIFLAGS=-Zkmiri-toml=/KMiri/tests/irq-v2-configs/$cfg timeout 1800 cargo osdk miri test" >"$log" 2>&1
    rc=$?; set -e
    echo "test=$t config=$cfg exit=$rc log=$log"
    echo "trace_sha256=$(grep '^KMiri IRQ:' "$log" | sha256sum | cut -d' ' -f1) lines=$(grep -c '^KMiri IRQ:' "$log")" ;;
  shell) docker exec -it "$CTR" bash -c "$ENV
cd /KMiri; exec bash" ;;
  *) sed -n '2,13p' "$0"; exit 2 ;;
esac
