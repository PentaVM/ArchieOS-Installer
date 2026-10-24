#!/usr/bin/env bash
# Verifies IRIS's F1–F4 work on the RTX 3070 WSL (ArchieOS-Operator-RTX3070-Test) and writes
# what it measured to _dev/verification/rtx3070/. Nothing it prints is a target met until
# it ran here: the cloud container that wrote this code has no GPU.
#
#   wsl -d ArchieOS-Operator-RTX3070-Test -u archie -- bash <repo>/scripts/iris/verify-rtx3070.sh \
#       --image /home/archie/iris/frame.jpg [--reference <dir> --reference-image <png>] \
#       [--weights <pt>] [--text-tower <dir>] [--flash] [--while-training]
#
# Steps:
#   1. CPU tests of archie-iris-runtime, archie-accel and archie-slink (release).
#   2. The CUDA build (CUDA 12.8 conda toolkit, CARGO_BUILD_JOBS=3: WSL has 7.7 GiB), with
#      flash-attention when --flash is given (a long nvcc build).
#   3. With --reference and --reference-image: the parity test against the Python
#      IRIS, on the CPU and then on the GPU in f32 (IRIS_DEVICE=cuda).
#   4. archie-iris-bench on the GPU in bf16, with VRAM sampled by nvidia-smi meanwhile.
#
# The GPU may be training Wanda's LNCM: the script stops unless --while-training says to
# share it (results are then marked so; the training's thermal pauses also skew timing).
set -euo pipefail

usage() { sed -n '2,20p' "$0"; exit 2; }

IMAGE= REFERENCE= REFERENCE_IMAGE= FLASH=0 WHILE_TRAINING=0
WEIGHTS=${IRIS_WEIGHTS:-/home/archie/iris/iris-beta21-step132000.pt}
TOWER=${IRIS_TEXT_TOWER:-/home/archie/iris/siglip2-text}
SHA256=${IRIS_SHA256:-}
while [[ $# -gt 0 ]]; do
  case "$1" in
    --image) IMAGE=$2; shift 2 ;;
    --reference) REFERENCE=$2; shift 2 ;;
    --reference-image) REFERENCE_IMAGE=$2; shift 2 ;;
    --weights) WEIGHTS=$2; shift 2 ;;
    --text-tower) TOWER=$2; shift 2 ;;
    --flash) FLASH=1; shift ;;
    --while-training) WHILE_TRAINING=1; shift ;;
    *) usage ;;
  esac
done
[[ -n $IMAGE && -f $IMAGE && -f $WEIGHTS && -d $TOWER ]] || usage
[[ -z $REFERENCE || -f $REFERENCE_IMAGE ]] || usage

REPO=$(cd "$(dirname "$0")/../.." && pwd)
CUDA=/home/archie/iris/cuda128
TARGET=$CUDA/targets/x86_64-linux
export CUDA_ROOT=$TARGET CUDA_PATH=$TARGET CUDA_HOME=$TARGET CUDA_TOOLKIT_ROOT_DIR=$TARGET
export PATH="$CUDA/bin:$HOME/.cargo/bin:/usr/lib/wsl/lib:/usr/local/bin:/usr/bin:/bin"
export NVCC_CCBIN=$CUDA/bin/x86_64-conda-linux-gnu-g++
export LD_LIBRARY_PATH="$CUDA/lib:/usr/lib/wsl/lib"
export LIBRARY_PATH="$CUDA/lib:/usr/lib/wsl/lib"
export CUDA_COMPUTE_CAP=86
export CARGO_BUILD_JOBS=3

training=0
if pgrep -f '[t]rain-local.sh' >/dev/null || pgrep -f '[a]rchie-wanda-lncm .*train' >/dev/null; then
  training=1
fi
if (( training && !WHILE_TRAINING )); then
  echo "Wanda's LNCM is training on the GPU. Run again when it finishes, or pass --while-training" >&2
  echo "to share the card (timings will then include its load and thermal pauses)." >&2
  exit 3
fi

stamp=$(date +%Y%m%d-%H%M%S)
out_dir="$REPO/_dev/verification/rtx3070"
mkdir -p "$out_dir"
out="$out_dir/iris-f1-f4-$stamp.json"
features=cuda
(( FLASH )) && features=flash-attn

cd "$REPO/core"
echo "[1/4] CPU tests"
cargo test --release --locked -p archie-iris-runtime -p archie-accel -p archie-slink

echo "[2/4] CUDA build ($features)"
cargo build --release --locked -p archie-iris-runtime --features "$features" --bins --tests

if [[ -n $REFERENCE ]]; then
  echo "[3/4] parity against the Python IRIS, CPU then GPU (f32)"
  IRIS_WEIGHTS=$WEIGHTS IRIS_REFERENCE=$REFERENCE IRIS_IMAGE=$REFERENCE_IMAGE \
    cargo test --release --locked -p archie-iris-runtime --features "$features" --test parity -- --ignored --nocapture \
    | tee "$out_dir/iris-parity-cpu-$stamp.log"
  IRIS_DEVICE=cuda IRIS_WEIGHTS=$WEIGHTS IRIS_REFERENCE=$REFERENCE IRIS_IMAGE=$REFERENCE_IMAGE \
    cargo test --release --locked -p archie-iris-runtime --features "$features" --test parity -- --ignored --nocapture \
    | tee "$out_dir/iris-parity-cuda-$stamp.log"
else
  echo "[3/4] parity skipped (no --reference)"
fi

echo "[4/4] bench on the GPU (bf16)"
vram_log=$(mktemp)
nvidia-smi --query-gpu=memory.used,temperature.gpu --format=csv,noheader,nounits -lms 200 >"$vram_log" &
sampler=$!
trap 'kill $sampler 2>/dev/null || true; rm -f "$vram_log"' EXIT
baseline=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | head -1)
sha_args=()
[[ -n $SHA256 ]] && sha_args=(--sha256 "$SHA256")
IRIS_PRECISION=bf16 ./target/release/archie-iris-bench --device cuda --weights "$WEIGHTS" "${sha_args[@]}" \
  --text-tower "$TOWER" --image "$IMAGE" --rounds 10 --live-seconds 15 --camera-fps 60 \
  --record-expert-usage --out "$out.bench"
kill $sampler 2>/dev/null || true
peak=$(cut -d, -f1 "$vram_log" | sort -n | tail -1)
hottest=$(cut -d, -f2 "$vram_log" | sort -n | tail -1)
python3 - "$out.bench" "$out" "$baseline" "$peak" "$hottest" "$training" "$features" <<'PY'
import json, sys
bench, out, baseline, peak, hottest, training, features = sys.argv[1:]
report = json.load(open(bench))
report["vram_mib"] = {"before": int(baseline), "peak": int(peak), "iris_peak": int(peak) - int(baseline)}
report["gpu_max_celsius"] = int(hottest)
report["shared_with_training"] = training == "1"
report["features"] = features
targets = {
    "F1 focused <= 90 ms": report["focused"]["median_ms"] <= 90,
    "F1 open <= 300 ms": report["open"] is None or report["open"]["median_ms"] <= 300,
    "F2 focused <= 50 ms": report["focused"]["median_ms"] <= 50,
    "frames answered >= 60/s": report["live"]["frames_answered_per_second"] >= 59.5,
    "answer p90 <= 16.7 ms": report["live"]["answer"]["p90_ms"] <= 16.7,
}
report["targets"] = targets
json.dump(report, open(out, "w"), indent=2)
print(json.dumps({k: report[k] for k in ("focused", "open", "multi4", "live", "vram_mib", "targets")}, indent=2))
PY
rm -f "$out.bench"
echo "written: $out"
