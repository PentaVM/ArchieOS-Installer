"""CPU fixture of owned-child cleanup; never imports Wanda or starts a process."""
import argparse
import ast
import hashlib
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace


class FixtureResourceError(Exception):
    pass


class FakeChild:
    def __init__(self):
        self.terminated = 0
        self.killed = 0

    def poll(self):
        return 0 if self.terminated or self.killed else None

    def terminate(self):
        self.terminated += 1

    def kill(self):
        self.killed += 1

    def wait(self, timeout):
        return 0


def audit(source):
    raw = source.read_bytes()
    tree = ast.parse(raw, filename=str(source))
    node = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == 'NativeChat')
    records = []
    for name in ('healthy_exit', 'post_load_resource_error', 'health_timeout'):
        child = FakeChild()
        clock = iter([0, 181] if name == 'health_timeout' else [0, 1])
        scope = {'Path': Path, '_free_port': lambda: 12345,
                 'native_environment': lambda config: {}, 'ResourceError': FixtureResourceError,
                 'time': SimpleNamespace(time=lambda: next(clock), sleep=lambda seconds: None),
                 'subprocess': SimpleNamespace(Popen=lambda *args, **kw: child, STDOUT=-2,
                                              TimeoutExpired=TimeoutError)}
        exec(compile(ast.Module(body=[node], type_ignores=[]), str(source), 'exec'), scope)
        with TemporaryDirectory() as folder:
            chat = scope['NativeChat'](SimpleNamespace(runtime_dir='fixture', model_path='unused'),
                                       Path(folder) / 'fixture.log')
            chat._get = lambda route: {'status': 'ok'}
            error = None
            try:
                with chat:
                    if name == 'post_load_resource_error':
                        raise FixtureResourceError('synthetic post-load budget violation')
            except FixtureResourceError as caught:
                error = str(caught)
            records.append({'case': name, 'error': error, 'terminate_calls': child.terminated,
                            'kill_calls': child.killed, 'log_closed': bool(chat.log and chat.log.closed),
                            'child_still_live_in_fixture': child.poll() is None})
            # Close our file and fake child after recording the real context-manager behavior.
            chat.__exit__(None, None, None)
    assert records[0]['terminate_calls'] == 1 and records[0]['log_closed']
    assert records[1]['terminate_calls'] == 1 and records[1]['log_closed']
    return {'scope': 'pure Python context-manager lifecycle fixture; zero native processes/GPU calls',
            'source_sha256': hashlib.sha256(raw).hexdigest(), 'cases': records,
            'live_worker_leak_incidence_measured': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    result = audit(args.source)
    args.out.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(result))
