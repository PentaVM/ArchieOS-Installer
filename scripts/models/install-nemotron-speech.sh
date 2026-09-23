#!/usr/bin/env bash
set -euo pipefail

runtime_commit=a5b6953c4a579a2bbd1c0913ad8a85c2a4d99953
model_sha256=d9a01898d2a611c8764e23a1c2f45e70bbd5a425dc4de93692ac951dd603812d
model_name=nemotron-speech-streaming-en-0.6b.q8_0.gguf
model_url="https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b/resolve/main/$model_name"
sentencepiece_commit=17d7580d6407802f85855d2cc9190634e2c95624

if [[ $EUID -eq 0 ]]; then
  prefix=${ARCHIE_NEMO_PREFIX:-/opt/archie/nemo-speech}
  model_dir=${ARCHIE_MODEL_DIR:-/archie/models}
  link_dir=/usr/local/bin
else
  prefix=${ARCHIE_NEMO_PREFIX:-$HOME/.local}
  model_dir=${ARCHIE_MODEL_DIR:-$HOME/.local/share/archie/models}
  link_dir=$HOME/.local/bin
fi
install -d "$prefix" "$model_dir" "$link_dir"

build_root=${ARCHIE_NEMO_BUILD_ROOT:-/var/tmp/archie-nemo-build}
install -d "$build_root"
trap 'rm -f "$model_dir/$model_name.part"' EXIT

# NeMo-Speech.cpp's upstream installer accepts a branch or tag for
# NEMO_SPEECH_SOURCE_REF and passes it to `git clone --branch`; a raw commit SHA
# (the reproducibility pin above) is not a valid branch. Clone the repository,
# check out the exact commit, then use NVIDIA's own configure/build presets.
if [[ ! -d "$build_root/source/.git" ]]; then
  git clone --filter=blob:none --no-checkout \
    https://github.com/NVIDIA/NeMo-Speech.cpp.git "$build_root/source"
fi
git -C "$build_root/source" checkout --detach "$runtime_commit"
initialize_submodule() {
  local submodule_path=$1
  if git -C "$build_root/source" config -f .gitmodules \
    --get-regexp '^submodule\..*\.path$' 2>/dev/null |
    awk '{print $2}' | grep -Fx "$submodule_path" >/dev/null 2>&1; then
    git -C "$build_root/source" submodule update --init --jobs 2 \
      --depth 1 --filter=blob:none "$submodule_path"
  fi
}
initialize_submodule ggml

(
  cd "$build_root/source"
  sentencepiece_source="$build_root/sentencepiece/source"
  sentencepiece_build="$build_root/sentencepiece/build"
  if [[ ! -d "$sentencepiece_source/.git" ]]; then
    git clone --filter=blob:none --no-checkout \
      https://github.com/google/sentencepiece.git "$sentencepiece_source"
  fi
  git -C "$sentencepiece_source" fetch --depth 1 origin "$sentencepiece_commit"
  git -C "$sentencepiece_source" checkout --detach "$sentencepiece_commit"
  cmake -G Ninja -S "$sentencepiece_source" -B "$sentencepiece_build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    "-DCMAKE_CXX_FLAGS=-include cstdint" \
    -DSPM_BUILD_TEST=OFF \
    -DSPM_ENABLE_SHARED=OFF \
    -DSPM_ENABLE_TCMALLOC=OFF
  CMAKE_BUILD_PARALLEL_LEVEL="${CMAKE_BUILD_PARALLEL_LEVEL:-2}" \
    cmake --build "$sentencepiece_build" --target sentencepiece-static
  install -d "$build_root/deps/sentencepiece/lib" \
    "$build_root/deps/sentencepiece/include" \
    "$build_root/deps/sentencepiece/share/licenses/nemo-speech/third_party/sentencepiece"
  install -m 0644 "$sentencepiece_build/src/libsentencepiece.a" \
    "$build_root/deps/sentencepiece/lib/libsentencepiece.a"
  install -m 0644 "$sentencepiece_source/src/sentencepiece_processor.h" \
    "$build_root/deps/sentencepiece/include/sentencepiece_processor.h"
  install -m 0644 "$sentencepiece_source/LICENSE" \
    "$build_root/deps/sentencepiece/share/licenses/nemo-speech/third_party/sentencepiece/LICENSE"
  install -m 0644 "$sentencepiece_source/third_party/absl/LICENSE" \
    "$build_root/deps/sentencepiece/share/licenses/nemo-speech/third_party/sentencepiece/absl-LICENSE"
  install -m 0644 "$sentencepiece_source/third_party/darts_clone/LICENSE" \
    "$build_root/deps/sentencepiece/share/licenses/nemo-speech/third_party/sentencepiece/darts-clone-LICENSE"
  install -m 0644 "$sentencepiece_source/third_party/protobuf-lite/LICENSE" \
    "$build_root/deps/sentencepiece/share/licenses/nemo-speech/third_party/sentencepiece/protobuf-lite-LICENSE"
  scripts/configure.sh cpu-server \
    "-DNEMO_SPEECH_DEPENDENCY_PREFIX=$build_root/deps" \
    -DNEMO_SPEECH_BUILD_NMT=OFF \
    -DNEMO_SPEECH_WITH_NMT=OFF \
    -DNEMO_SPEECH_BUILD_HTTP=OFF
  CMAKE_BUILD_PARALLEL_LEVEL="${CMAKE_BUILD_PARALLEL_LEVEL:-2}" \
    cmake --build --preset cpu-server
  cmake --install "build/cpu-server" --prefix "$build_root/install"
)

test -x "$build_root/install/bin/nemo-speech"
next="$prefix.new"
previous="$prefix.old"
rm -rf "$next" "$previous"
mv "$build_root/install" "$next"
if [[ -e $prefix ]]; then mv "$prefix" "$previous"; fi
if ! mv "$next" "$prefix"; then
  if [[ -e $previous ]]; then mv "$previous" "$prefix"; fi
  echo "could not activate NeMo-Speech; previous install restored" >&2
  exit 1
fi
rm -rf "$previous"
ln -sfn "$prefix/bin/nemo-speech" "$link_dir/nemo-speech"

if [[ -f "$model_dir/$model_name" ]] &&
  printf '%s  %s\n' "$model_sha256" "$model_dir/$model_name" | sha256sum --check --status; then
  echo "Existing Nemotron model checksum matches the pinned artifact"
else
  curl --fail --location --proto '=https' --tlsv1.2 "$model_url" \
    -o "$model_dir/$model_name.part"
  printf '%s  %s\n' "$model_sha256" "$model_dir/$model_name.part" | sha256sum --check --status
  mv "$model_dir/$model_name.part" "$model_dir/$model_name"
fi
ln -sfn "$prefix/bin/nemo-speech" "$link_dir/nemo-speech"
printf 'runtime_commit=%s\nmodel_sha256=%s\n' "$runtime_commit" "$model_sha256" \
  > "$model_dir/nemotron-speech-install.metadata"
echo "Nemotron Speech installed locally at $model_dir/$model_name"
