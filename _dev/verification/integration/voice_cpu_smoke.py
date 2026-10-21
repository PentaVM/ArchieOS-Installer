#!/usr/bin/env python3
"""Real CPU ASR/Piper through the CLI; scripted loopback reasoning, no GPU/room claim.

Requires existing models and Piper's venv. Downloads nothing, starts no production
service and bounds the entire child process group. Output is synthetic test audio.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import tempfile
import threading
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def bounded(command, env, seconds):
    process = subprocess.Popen(command, env=env, start_new_session=True,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        output, _ = process.communicate(timeout=seconds)
        return process.returncode, output
    finally:
        # Kill only this test's process group, including lingering Piper children.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--asr', type=Path, required=True)
    parser.add_argument('--piper-model', type=Path, required=True)
    parser.add_argument('--python', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    bridge = root / 'scripts/models/piper-json-bridge.py'
    args.output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CUDA_VISIBLE_DEVICES='', PIPER_THREADS='2', OMP_NUM_THREADS='2')
    # Keep model registries and guides from selecting an unrelated/cloud endpoint.
    env.pop('ARCHIE_VOICE_GUIDE', None)
    env['LD_LIBRARY_PATH'] = '/opt/archie/nemo-speech/lib:' + env.get('LD_LIBRARY_PATH', '')
    affinity = sorted(os.sched_getaffinity(0))[:2]
    prefix = ['taskset', '-c', ','.join(map(str, affinity)), 'nice', '-n', '10']
    requests = []

    class Reply(BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers.get('Content-Length', 0))
            if self.path != '/v1/chat/completions' or length > 65536:
                self.send_error(400)
                return
            requests.append(json.loads(self.rfile.read(length)))
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            for delta, reason in [({'content': 'Estoy escuchando. Puedes continuar. '}, None), ({}, 'stop')]:
                payload = {'choices': [{'index': 0, 'delta': delta, 'finish_reason': reason}]}
                self.wfile.write(('data: ' + json.dumps(payload) + '\n\n').encode())
            self.wfile.write(b'data: [DONE]\n\n')
            self.wfile.flush()

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Reply)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='archie-voice-smoke-') as directory:
            temp = Path(directory)
            env['ARCHIE_MODELS_DIR'] = str(temp / 'empty-models')
            wrapper = temp / 'piper'
            wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(str(args.python)) + ' ' +
                               shlex.quote(str(bridge)) + ' "$@"\n')
            wrapper.chmod(0o700)
            audio_in = args.output / 'input.wav'
            audio_out = args.output / 'reply.wav'
            request = json.dumps({'text': 'Archie, dime una frase corta.', 'output_file': str(audio_in)})
            code = ('import subprocess,sys; p=subprocess.run(sys.argv[1:], input=' + repr(request + '\n') +
                    ', text=True, check=True);')
            status, log = bounded(prefix + [str(args.python), '-c', code, str(wrapper),
                                           '--model', str(args.piper_model)], env, 30)
            (args.output / 'input-synthesis.log').write_text(log)
            if status:
                raise RuntimeError('input Piper synthesis failed; see input-synthesis.log')
            command = prefix + [str(args.binary), 'converse', '--grade', 'internal', '--asr-model', str(args.asr),
                                '--voice-engine', 'piper', '--piper', str(wrapper), '--piper-model',
                                str(args.piper_model), '--wav-in', str(audio_in), '--wav-out', str(audio_out),
                                '--endpoint', f'http://127.0.0.1:{server.server_port}/v1', '--model', 'cpu-smoke',
                                '--always', '--timeout-seconds', '15', '--threads', '2']
            status, log = bounded(command, env, 120)
            (args.output / 'conversation.log').write_text(log)
            if status or not requests or not audio_out.exists():
                raise RuntimeError(f'CPU conversation failed (exit {status}, requests {len(requests)}); see conversation.log')
            with wave.open(str(audio_out)) as audio:
                frames, rate = audio.getnframes(), audio.getframerate()
            if not frames or 'first_segment' not in log or 'complete' not in log:
                raise RuntimeError('missing nonempty audio or terminal timing evidence')
            result = {'status': 'pass', 'cpu_affinity': affinity, 'requests': len(requests),
                      'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                      'output_frames': frames, 'sample_rate': rate, 'reasoning': 'scripted-loopback',
                      'microphone_or_speaker': False, 'gpu': False}
            (args.output / 'receipt.json').write_text(json.dumps(result, indent=2) + '\n')
            print(json.dumps(result))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
