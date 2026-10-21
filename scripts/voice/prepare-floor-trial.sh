#!/bin/sh
# Install only a separately named trial, never replace the workstation/GUI launcher.
set -eu
if [ "$#" -ne 2 ]; then
    echo 'usage: prepare-floor-trial.sh <built-archie-voice> <new-trial-directory>' >&2
    exit 2
fi
binary=$(realpath "$1")
destination=$2
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
if [ -e "$destination" ]; then
    echo "Trial destination already exists; choose a new directory: $destination" >&2
    exit 1
fi
test -x "$binary"
revision=${ARCHIE_VOICE_SOURCE_REV:-}
if [ -z "$revision" ]; then
    revision=$(git -C "$root" rev-parse HEAD) || {
        echo 'Set ARCHIE_VOICE_SOURCE_REV from the host Git when preparing a Windows worktree in WSL.' >&2
        exit 1
    }
fi
case "$revision" in
    ''|*[!0-9a-f]*) echo 'Source revision must be a full lowercase Git SHA.' >&2; exit 1 ;;
esac
test "${#revision}" -eq 40 || { echo 'Source revision must be a full Git SHA.' >&2; exit 1; }
mkdir -p "$destination"
cp "$binary" "$destination/archie-voice"
cp "$root/scripts/models/piper-json-bridge.py" "$destination/piper-json-bridge.py"
printf '%s\n' "$revision" > "$destination/source-base.txt"
git -C "$root" diff --stat > "$destination/source-diff.txt" 2>/dev/null ||
    printf 'Host Git worktree; source revision supplied by ARCHIE_VOICE_SOURCE_REV.\n' > "$destination/source-diff.txt"
cat > "$destination/piper" <<'PIPER'
#!/bin/sh
set -eu
base=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
exec "${ARCHIE_PIPER_PYTHON:-/opt/archie/iris-venv/bin/python}" "$base/piper-json-bridge.py" "$@"
PIPER
cat > "$destination/run" <<'RUN'
#!/bin/sh
set -eu
base=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
exec 9>"$base/session.lock"
flock -n 9 || { echo 'Esta copia de prueba ya tiene una sesión abierta.' >&2; exit 1; }
export CUDA_VISIBLE_DEVICES=''
export OMP_NUM_THREADS=2 PIPER_THREADS=2
export ARCHIE_MODELS_DIR="$base/empty-models"
export LD_LIBRARY_PATH="/opt/archie/nemo-speech/lib:${LD_LIBRARY_PATH:-}"
unset ARCHIE_VOICE_GUIDE
asr=${ARCHIE_ASR_MODEL:-/var/lib/archie/models/asr/nemotron-3.5-asr-streaming-0.6b.q8_0.gguf}
spanish=${ARCHIE_PIPER_ES:-/var/lib/archie/models/piper/es_MX-ald-medium.onnx}
english=${ARCHIE_PIPER_EN:-/var/lib/archie/models/piper/en_US-john-medium.onnx}
for file in "$base/archie-voice" "$asr" "$spanish" "$english"; do
    test -r "$file" || { echo "Falta el archivo: $file" >&2; exit 1; }
done
# Restrict this voice process and its children to two of its permitted CPU cores.
cores=$("${ARCHIE_PIPER_PYTHON:-/opt/archie/iris-venv/bin/python}" -c 'import os; print(",".join(map(str, sorted(os.sched_getaffinity(0))[:2])))')
exec taskset -c "$cores" nice -n 10 "$base/archie-voice" converse \
    --grade internal --asr-model "$asr" --voice-engine piper \
    --piper "$base/piper" --piper-model "$spanish" --piper-model-en "$english" \
    --listen-language es-US --threads 2 --timeout-seconds 600 "$@"
RUN
chmod 700 "$destination/archie-voice" "$destination/piper" "$destination/run"
mkdir "$destination/empty-models"
sha256sum "$destination/archie-voice" "$destination/piper-json-bridge.py" > "$destination/SHA256SUMS"
printf 'Trial prepared: %s\n' "$destination"
