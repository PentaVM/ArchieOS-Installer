#!/usr/bin/env bash
# Wanda LNCM: GPU training stage on the RTX 3070, built against the CUDA 12.8 toolkit IRIS
# already uses (/home/archie/iris/cuda128). The Windows driver 572.16 supports CUDA up to
# 12.8, so Arch's cuda 13 package cannot run on it (CUBLAS_STATUS_NOT_INITIALIZED).
# Run from Windows:  wsl -d ArchieOS-Operator-RTX3070-Test -- bash /mnt/c/Users/Penta/Desktop/ArchieOS/wanda-train-cuda128.sh
set -euo pipefail
CUDA=/home/archie/iris/cuda128
REPO=/mnt/c/Users/Penta/Desktop/ArchieOS-Nvidia-wanda

# A conda toolkit: nvcc in bin/, headers and libraries under targets/x86_64-linux/.
TARGET=$CUDA/targets/x86_64-linux
export CUDA_ROOT=$TARGET CUDA_PATH=$TARGET CUDA_HOME=$TARGET CUDA_TOOLKIT_ROOT_DIR=$TARGET
# /usr/lib/wsl/lib holds the Windows driver's nvidia-smi and libcuda.
export PATH="$CUDA/bin:$HOME/.cargo/bin:/usr/lib/wsl/lib:/usr/local/bin:/usr/bin:/bin"
# nvcc 12.8 accepts GCC up to 14; the toolkit ships its own g++ 14.
export NVCC_CCBIN=$CUDA/bin/x86_64-conda-linux-gnu-g++
export LD_LIBRARY_PATH="$CUDA/lib:/usr/lib/wsl/lib"
export LIBRARY_PATH="$CUDA/lib:/usr/lib/wsl/lib"
export CUDA_COMPUTE_CAP=86
export STAGES=train WANDA_PRESET=small
# WSL has 7.7 GiB: a full-width cargo build next to the LLM got IRIS OOM-killed (2026-10-20).
export CARGO_BUILD_JOBS=3

# Binaries built against CUDA 13 cannot load on this driver: rebuild them once.
STAMP="$REPO/core/target/.cuda-toolkit"
if [[ "$(cat "$STAMP" 2>/dev/null)" != "$CUDA" ]]; then
  echo "[wanda] rebuilding against $CUDA (CUDA 12.8)"
  rm -rf "$REPO/core/target/release"
  mkdir -p "$REPO/core/target" && echo "$CUDA" > "$STAMP"
fi

cd "$REPO"
exec training/wanda_lncm/train-local.sh
