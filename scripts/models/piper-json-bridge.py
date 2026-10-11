#!/usr/bin/env python3
"""Keep Piper's Python voice loaded and adapt newline JSON requests from archie-voice."""

import argparse
import json
import os
import sys
import tempfile
import wave
from pathlib import Path


def main() -> int:
    # Let the caller select a virtualenv or an external Piper package directory.
    for entry in reversed(os.environ.get("PYTHONPATH", "").split(os.pathsep)):
        if entry:
            sys.path.insert(0, entry)

    import onnxruntime
    from piper import PiperVoice

    # Keep CPU inference bounded so voice synthesis does not compete with training workers.
    original_session_options = onnxruntime.SessionOptions

    def bounded_session_options():
        options = original_session_options()
        options.intra_op_num_threads = int(os.environ.get("PIPER_THREADS", "2"))
        options.inter_op_num_threads = 1
        return options

    onnxruntime.SessionOptions = bounded_session_options

    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--json-input", action="store_true")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()

    model = Path(args.model)
    config_path = Path(str(model) + ".json")
    config = json.loads(config_path.read_text(encoding="utf-8"))
    # Some published Piper cards serialize the enum's Python name rather than its value.
    if config.get("phoneme_type") == "PhonemeType.ESPEAK":
        config["phoneme_type"] = "espeak"
    normalized_config = Path(tempfile.gettempdir()) / f"{model.stem}-{os.getpid()}.piper.json"
    normalized_config.write_text(json.dumps(config), encoding="utf-8")
    voice = PiperVoice.load(model, config_path=normalized_config, use_cuda=False)

    for line in sys.stdin:
        request = json.loads(line)
        output_file = request["output_file"]
        with wave.open(output_file, "wb") as wav_file:
            voice.synthesize_wav(request["text"], wav_file)
        print(output_file, flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # Piper's stderr is owned by archie-voice.
        print(f"piper-json-bridge: {error}", file=sys.stderr, flush=True)
        raise SystemExit(1)
