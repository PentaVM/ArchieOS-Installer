"""Owned loopback llama-server for evaluation generation (base or base + LoRA candidate).

Started and stopped by the worker that owns the GPU, so the thermal guard's pause applies
between requests. Greedy decoding, thinking disabled unless a request asks for it.
"""
import json
import socket
import subprocess
import time
import urllib.request
from pathlib import Path
from .contracts import ResourceError
from .native import native_environment


def _free_port():
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        return probe.getsockname()[1]


class NativeChat:
    def __init__(self, config, log_path, lora=None, context=8192):
        self.config, self.log_path, self.lora, self.context = config, Path(log_path), lora, context
        self.port, self.process, self.log = _free_port(), None, None

    def __enter__(self):
        command = [self.config.runtime_dir + '/llama-server', '-m', self.config.model_path, '-ngl', '99',
                   '-c', str(self.context), '--parallel', '1', '--host', '127.0.0.1', '--port', str(self.port),
                   '--no-webui',
                   # The default host prompt cache (8 GiB) and 32 per-slot checkpoints of the hybrid
                   # model's recurrent state outgrow this 7.9 GiB guest (measured: +0.5 MiB/request up
                   # to 6.2 GiB RSS). One shared context per question set needs far less.
                   '--cache-ram', '1024', '--ctx-checkpoints', '4']
        if self.lora:
            command += ['--lora', str(self.lora)]
        self.log = self.log_path.open('a')
        self.process = subprocess.Popen(command, env=native_environment(self.config), stdout=self.log,
                                        stderr=subprocess.STDOUT)
        deadline = time.time() + 180
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise ResourceError('llama-server exited during start; see ' + str(self.log_path))
            try:
                if self._get('/health').get('status') == 'ok':
                    return self
            except OSError:
                pass
            time.sleep(1)
        raise ResourceError('llama-server did not become healthy')

    def __exit__(self, *exc):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill(); self.process.wait(timeout=10)
        if self.log:
            self.log.close()

    def _get(self, route):
        with urllib.request.urlopen(f'http://127.0.0.1:{self.port}{route}', timeout=10) as response:
            return json.load(response)

    def _post(self, route, payload, timeout=600):
        request = urllib.request.Request(f'http://127.0.0.1:{self.port}{route}', data=json.dumps(payload).encode(),
                                         headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return json.load(response)

    def chat(self, messages, max_tokens=256, thinking=False, seed=17):
        answer = self._post('/v1/chat/completions', {
            'messages': messages, 'temperature': 0, 'max_tokens': max_tokens, 'seed': seed,
            'cache_prompt': True, 'chat_template_kwargs': {'enable_thinking': thinking}})
        choice = answer['choices'][0]
        return {'text': choice['message'].get('content') or '', 'finish': choice.get('finish_reason'),
                'tokens_in': answer.get('usage', {}).get('prompt_tokens'),
                'tokens_out': answer.get('usage', {}).get('completion_tokens')}

    def _complete(self, prompt, n_predict, stop, seed):
        out = self._post('/completion', {'prompt': prompt, 'n_predict': n_predict, 'temperature': 0, 'seed': seed,
                                         'stop': stop, 'cache_prompt': True})
        stopped_word = out.get('stop_type') == 'word' or out.get('stopped_word')
        return out.get('content', ''), int(out.get('tokens_predicted', 0)), bool(stopped_word)

    def think(self, messages, budget=512, min_think=0, max_waits=0, answer_tokens=64, seed=17):
        """s1-style budget forcing (arXiv:2501.19393) on the model's own chat template.

        The model reasons inside <think>. At `budget` tokens the thinking is cut and the
        end-of-thinking delimiter is appended; if it stops before `min_think` tokens, the
        delimiter is replaced by "Wait" (at most `max_waits` times). The answer is then
        decoded greedily after the delimiter.
        """
        prompt = render_chat(messages) + '<think>\n'
        thinking, used, waits = '', 0, 0
        while used < budget:
            text, n, stopped = self._complete(prompt + thinking, budget - used, ['</think>'], seed)
            thinking += text; used += n
            if not stopped:
                break                                   # budget exhausted: force the end
            if used < min_think and waits < max_waits:
                thinking += 'Wait'; waits += 1
                continue
            break
        closed = prompt + thinking.rstrip() + '\n</think>\n\n'
        answer, n_answer, _ = self._complete(closed, answer_tokens, ['<|im_end|>'], seed)
        return {'text': answer.strip(), 'thinking_tokens': used, 'waits': waits,
                'forced_end': used >= budget, 'tokens_out': used + n_answer, 'thinking': thinking}


def render_chat(messages):
    """The base model's chat template (no tools), up to the assistant turn header."""
    parts = []
    for message in messages:
        if message['role'] not in ('system', 'user'):
            raise ValueError('only system/user turns are rendered for evaluation prompts')
        parts.append(f"<|im_start|>{message['role']}\n{message['content'].strip()}<|im_end|>\n")
    return ''.join(parts) + '<|im_start|>assistant\n'
