#!/bin/sh
# Archie, whole: the stage on a screen, and a voice that listens, answers and acts on it.
#
#   NEBIUS_TOKEN_FACTORY_KEY=... display/stage/run-archie.sh public
#
# The argument is how private what you say is. It has no default, because what a person
# says to their computer is theirs: `public` lets it reach Nemotron on Nebius, whose
# retention nobody has verified; anything else is refused before it leaves the machine.
#
# Say "Archie" to be heard. Applications open only as the stage policy spells them out.
# Move the mouse over the stage and Archie stops acting until you let go.
#
# Environment (all optional):
#   STAGE, STAGE_CLI, ARCHIE_VOICE   binaries                  default: from PATH
#   POLICY        stage policy                     default: core/crates/archie-stage/policy/apps.toml
#   ASR_MODEL     Nemotron 3.5 ASR GGUF            default: the one `nemo-speech pull` cached
#   PIPER, PIPER_PYTHON, PIPER_MODEL, PIPER_MODEL_EN   Piper adapter/runtime and voices
#                 see scripts/models/install-piper-voices.sh
#   HOST_WAYLAND  session the stage opens in       default: $WAYLAND_DISPLAY
#                 (WSLg: /mnt/wslg/runtime-dir/wayland-0)
#   SCREEN        which screen the stage takes     default: the last one
#   RUN_DIR       key, sockets, logs               default: /tmp/archie-run
#   LISTEN_LANGUAGE  what the recogniser listens for: es-US, en-US or auto  default: es-US
#                 (auto: either, and Archie answers in the language it was spoken to)
#   HEADPHONES=1  keep listening while Archie speaks, so it can be interrupted
#   VOICE_ENGINE  piper (default) or magpie, NVIDIA's Magpie TTS: then TTS_MODEL, CODEC_MODEL,
#                 TTS_TOKENIZER (the extracted .nemo) and TTS_VOICE (e.g. John) name its files
#   MODEL_HEARS=1 the reasoning model is sent the sound of each utterance (Nemotron Omni)
#   ALWAYS=1      answer whatever is heard, named or not (one person alone)
#   SESSION_LOG   also write the session there, each line with its time
set -eu

GRADE=${1:-}
case "$GRADE" in
    public|internal) ;;
    *) echo "run-archie: say how private this is: run-archie.sh public (or internal)" >&2; exit 64 ;;
esac
# ARCHIE_VOICE_ENDPOINT names a reasoning model on this machine; then no key is needed.
[ -n "${NEBIUS_TOKEN_FACTORY_KEY:-}" ] || [ -n "${ARCHIE_VOICE_ENDPOINT:-}" ] || {
    echo "run-archie: set NEBIUS_TOKEN_FACTORY_KEY, or ARCHIE_VOICE_ENDPOINT for a local model" >&2
    exit 64
}

# Accessibility needs a session bus, and the stage's clients inherit it.
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ] && command -v dbus-run-session >/dev/null; then
    exec dbus-run-session -- "$0" "$@"
fi

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
STAGE=${STAGE:-archie-stage}
STAGE_CLI=${STAGE_CLI:-archie-stage-cli}
ARCHIE_VOICE=${ARCHIE_VOICE:-archie-voice}
POLICY=${POLICY:-$REPO/core/crates/archie-stage/policy/apps.toml}
ASR_MODEL=${ASR_MODEL:-$(ls "$HOME"/.cache/nemo-speech/models/nvidia/nemotron-3.5-asr-streaming-0.6b/*/*.gguf 2>/dev/null | head -n 1)}
PIPER_PYTHON=${PIPER_PYTHON:-python3}
PIPER=${PIPER:-$REPO/scripts/models/piper-json-bridge.py}
PIPER_VOICES=${PIPER_VOICES:-$REPO/models-cache/piper}
# Male voices, since 2026-10-25 (scripts/models/install-piper-voices.sh); else the pair
# that was heard best.
default_voice() { if [ -f "$PIPER_VOICES/$1.onnx" ]; then echo "$PIPER_VOICES/$1.onnx"; else echo "$PIPER_VOICES/$2.onnx"; fi; }
PIPER_MODEL=${PIPER_MODEL:-$(default_voice es_MX-ald-medium es_MX-claude-high)}
PIPER_MODEL_EN=${PIPER_MODEL_EN:-$(default_voice en_US-john-medium en_US-kristin-medium)}
HOST_WAYLAND=${HOST_WAYLAND:-${WAYLAND_DISPLAY:-}}
RUN_DIR=${RUN_DIR:-/tmp/archie-run}
SOCKET_NAME=archie-stage

[ -n "$HOST_WAYLAND" ] || { echo "run-archie: set HOST_WAYLAND to the Wayland session to open the stage in" >&2; exit 1; }

# The stage gets its own runtime directory, and PulseAudio finds its server through that
# variable: name the person's sound server before it changes, or the voice goes deaf.
if [ -z "${PULSE_SERVER:-}" ]; then
    if [ -S /mnt/wslg/PulseServer ]; then
        export PULSE_SERVER=unix:/mnt/wslg/PulseServer
    elif [ -n "${XDG_RUNTIME_DIR:-}" ] && [ -S "$XDG_RUNTIME_DIR/pulse/native" ]; then
        export PULSE_SERVER="unix:$XDG_RUNTIME_DIR/pulse/native"
    fi
fi
[ -n "$ASR_MODEL" ] && [ -f "$ASR_MODEL" ] || { echo "run-archie: no ASR model; run nemo-speech pull nemotron-3.5 or set ASR_MODEL" >&2; exit 1; }
VOICE_ENGINE=${VOICE_ENGINE:-piper}
if [ "$VOICE_ENGINE" = magpie ]; then
    for f in "${TTS_MODEL:-}" "${CODEC_MODEL:-}"; do
        [ -f "$f" ] || { echo "run-archie: Magpie needs TTS_MODEL and CODEC_MODEL; missing ${f:-a path}" >&2; exit 1; }
    done
    [ -d "${TTS_TOKENIZER:-}" ] || { echo "run-archie: Magpie needs TTS_TOKENIZER, its extracted .nemo" >&2; exit 1; }
else
[ -f "$PIPER_MODEL" ] || { echo "run-archie: no Spanish Piper model at $PIPER_MODEL; run scripts/models/install-piper-voices.sh" >&2; exit 1; }
fi
if [ "$VOICE_ENGINE" = piper ] && [ "$PIPER" = "$REPO/scripts/models/piper-json-bridge.py" ]; then
    PIPER_PYTHON_BIN=$(command -v "$PIPER_PYTHON" 2>/dev/null || true)
    [ -n "$PIPER_PYTHON_BIN" ] || { echo "run-archie: Python runtime not found: $PIPER_PYTHON" >&2; exit 1; }
    PATH=$(dirname "$PIPER_PYTHON_BIN"):$PATH
    export PATH
    "$PIPER_PYTHON_BIN" -c 'import onnxruntime, piper' 2>/dev/null || {
        echo "run-archie: install Piper Python and ONNX Runtime in $PIPER_PYTHON" >&2
        exit 1
    }
fi

mkdir -p "$RUN_DIR/xdg"
chmod 700 "$RUN_DIR" "$RUN_DIR/xdg"
rm -f "$RUN_DIR/control.sock" "$RUN_DIR/xdg/$SOCKET_NAME"
[ -f "$RUN_DIR/key" ] || "$STAGE_CLI" keygen "$RUN_DIR/key" >/dev/null

if [ -x /usr/lib/at-spi-bus-launcher ]; then
    /usr/lib/at-spi-bus-launcher --launch-immediately >/dev/null 2>&1 &
fi

set -- --socket-name "$SOCKET_NAME" --control "$RUN_DIR/control.sock" --key-file "$RUN_DIR/key"
[ -n "${SCREEN:-}" ] && set -- "$@" --screen "$SCREEN"
XDG_RUNTIME_DIR="$RUN_DIR/xdg" WAYLAND_DISPLAY="$HOST_WAYLAND" QT_QPA_PLATFORM=wayland \
    "$STAGE" "$@" > "$RUN_DIR/stage.log" 2>&1 &
STAGE_PID=$!
trap 'kill "$STAGE_PID" 2>/dev/null || true' EXIT INT TERM

for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    [ -S "$RUN_DIR/control.sock" ] && [ -S "$RUN_DIR/xdg/$SOCKET_NAME" ] && break
    sleep 0.5
done
[ -S "$RUN_DIR/xdg/$SOCKET_NAME" ] || { echo "run-archie: the stage did not start; see $RUN_DIR/stage.log" >&2; exit 1; }

set -- converse --grade "$GRADE" --asr-model "$ASR_MODEL" \
    --socket "$RUN_DIR/control.sock" --key-file "$RUN_DIR/key" --policy "$POLICY" \
    --stage-display "$SOCKET_NAME" --app-log "$RUN_DIR/apps.log"
if [ "$VOICE_ENGINE" = magpie ]; then
    # One multilingual voice; each reply is spoken in the language it is written in.
    set -- "$@" --voice-engine magpie --tts-model "$TTS_MODEL" --codec-model "$CODEC_MODEL" \
        --tts-tokenizer "$TTS_TOKENIZER" --tts-language "${TTS_LANGUAGE:-es-US}"
    [ -n "${TTS_VOICE:-}" ] && set -- "$@" --voice "$TTS_VOICE"
else
    set -- "$@" --voice-engine piper --piper "$PIPER" --piper-model "$PIPER_MODEL"
    if [ -f "$PIPER_MODEL_EN" ]; then
        set -- "$@" --piper-model-en "$PIPER_MODEL_EN"
    else
        echo "run-archie: English Piper voice is not installed; using Spanish for every reply" >&2
    fi
fi
[ "${MODEL_HEARS:-0}" = 1 ] && set -- "$@" --model-hears
[ -n "${LISTEN_LANGUAGE:-}" ] && set -- "$@" --listen-language "$LISTEN_LANGUAGE"
[ "${HEADPHONES:-0}" = 1 ] && set -- "$@" --headphones
[ "${ALWAYS:-0}" = 1 ] && set -- "$@" --always

# Two thread pools share this process's cores; spinning ones starve each other.
# Applications opened by voice inherit the stage's runtime directory and software rendering.
voice() {
    XDG_RUNTIME_DIR="$RUN_DIR/xdg" OMP_WAIT_POLICY=PASSIVE \
        QT_QUICK_BACKEND=software LIBGL_ALWAYS_SOFTWARE=1 QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 \
        "$ARCHIE_VOICE" "$@"
}
[ -n "${SESSION_LOG:-}" ] || { voice "$@"; exit; }

# The session is kept as the window shows it, each line with its wall-clock time: what was
# heard, what was said, the floor's decisions and the timings. Only the owner can read it.
mkdir -p "$(dirname "$SESSION_LOG")"
chmod 700 "$(dirname "$SESSION_LOG")"
(umask 077 && : >>"$SESSION_LOG")
{
    echo "# archie session $(date -Iseconds)"
    echo "# grade=$GRADE model=${ARCHIE_VOICE_MODEL:-default} endpoint=${ARCHIE_VOICE_ENDPOINT:-nebius} headphones=${HEADPHONES:-0} always=${ALWAYS:-0}"
} >>"$SESSION_LOG"
voice "$@" 2>&1 | {
    # Ctrl+C reaches the voice too; this reader keeps writing until the voice has stopped.
    trap '' INT TERM
    while IFS= read -r line; do
        printf '%s\n' "$line"
        printf '%s %s\n' "$(date +%T.%3N)" "$line" >>"$SESSION_LOG"
    done
}
echo "# ended $(date -Iseconds)" >>"$SESSION_LOG"
