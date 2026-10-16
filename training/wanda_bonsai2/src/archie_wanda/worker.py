"""Queue-driven local worker: native extraction ("prepare") and LoRA ("train") jobs.

Runs independently of any chat session or subscription. One owned process per run
root (flock). Steering happens only through files under the run root:
queue/*.json job specs (run in name order) and control.json
{"command": "run" | "pause" | "stop"}. Pause/stop take effect at an optimizer-step
boundary after an atomic checkpoint, so a restarted worker resumes the same job.
This worker has no Core authority and no moral judge; it optimizes the admitted data.
"""
import fcntl
import json
import os
import random
import re
import shutil
import time
import traceback
from dataclasses import asdict
from pathlib import Path
import numpy as np
from .contracts import BASE_SHA, AdmissionError, ResourceError, digest
from .resources import preflight
from .verification import atomic_json

MAX_STARTS = 3


class Interrupted(Exception):
    """Stop/cutoff requested; the job is checkpointed and stays queued."""


def now_ms():
    return int(time.time()*1000)


def atomic_torch_save(obj, path):
    import torch
    path = Path(path)
    temporary = path.with_name(path.name+'.partial')
    with temporary.open('wb') as stream:
        torch.save(obj, stream)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def prune_reference_outputs(root):
    """Training needs only prefix rows; keep digests of the dropped native references."""
    root = Path(root)
    info = json.loads((root/'receipt.json').read_text())
    pruned = info.setdefault('pruned', {})
    for name in ('logits.f32', 'norm.f32'):
        if name in info['files']:
            pruned[name] = info['files'].pop(name)
            (root/name).unlink(missing_ok=True)
    atomic_json(root/'receipt.json', info)


class Worker:
    def __init__(self, config, root, headers, cutoff_epoch):
        self.config, self.root, self.headers, self.cutoff = config, Path(root), headers, cutoff_epoch
        for name in ('queue', 'queue/running', 'queue/done', 'queue/failed', 'runs', 'datasets', 'cache'):
            (self.root/name).mkdir(parents=True, exist_ok=True)
        self.state, self.job, self.binary, self.lock = 'starting', None, None, None
        self.boot_id = Path('/proc/sys/kernel/random/boot_id').read_text().strip()

    # ---- lifecycle -------------------------------------------------------------------
    def heartbeat(self, **extra):
        atomic_json(self.root/'worker.json', {
            'format_version': 1, 'pid': os.getpid(), 'boot_id': self.boot_id, 'state': self.state,
            'job': self.job, 'updated_ms': now_ms(), 'cutoff_epoch': self.cutoff, **extra})

    def control(self):
        try:
            command = json.loads((self.root/'control.json').read_text()).get('command', 'run')
        except (OSError, ValueError):
            return 'run'
        return command if command in ('run', 'pause', 'stop') else 'run'

    def stopping(self):
        return time.time() >= self.cutoff or self.control() == 'stop'

    def acquire(self):
        self.lock = open(self.root/'worker.lock', 'w')
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ResourceError('another owned worker holds this run root') from error
        self.lock.write(str(os.getpid())); self.lock.flush()

    def run_forever(self):
        self.acquire()
        for stale in sorted((self.root/'queue/running').glob('*.json')):
            stale.replace(self.root/'queue'/stale.name)       # interrupted by crash/restart: resume
        while not self.stopping():
            if self.control() == 'pause':
                self.state = 'paused'; self.heartbeat(); time.sleep(20); continue
            jobs = sorted((self.root/'queue').glob('*.json'))
            if not jobs:
                self.state = 'idle'; self.heartbeat(); time.sleep(30); continue
            self.run_job(jobs[0])
        self.state = 'stopped'; self.job = None; self.heartbeat(reason='cutoff' if time.time() >= self.cutoff else 'stop')

    def run_job(self, path):
        running = self.root/'queue/running'/path.name
        path.replace(running)
        spec = json.loads(running.read_text())
        spec['starts'] = spec.get('starts', 0)+1
        atomic_json(running, spec)
        self.job = spec.get('job_id', path.stem)
        if spec['starts'] > MAX_STARTS:
            spec['last_error'] = spec.get('last_error', 'repeated crash/restart')
            atomic_json(self.root/'queue/failed'/path.name, spec); running.unlink(); return
        try:
            self.state = 'running'; self.heartbeat()
            if spec['type'] == 'prepare':
                result = self.prepare(spec)
            elif spec['type'] == 'train':
                result = self.train(spec)
            else:
                raise AdmissionError('unknown job type')
            spec['result'], spec['finished_ms'] = result, now_ms()
            atomic_json(self.root/'queue/done'/path.name, spec); running.unlink()
        except Interrupted:
            running.replace(self.root/'queue'/path.name)
        except Exception as error:
            spec['last_error'] = ''.join(traceback.format_exception(error))[-4000:]
            # Another app using the GPU/RAM/disk is a reason to wait, not a job failure;
            # the job resumes from its last atomic checkpoint once headroom returns.
            waiting = isinstance(error, ResourceError) or type(error).__name__ == 'OutOfMemoryError'
            if waiting:
                spec['starts'] -= 1
            destination = self.root/('queue' if waiting else 'queue/failed')/path.name
            atomic_json(destination, spec); running.unlink()
            if waiting:
                self.state = 'waiting_resources'; self.heartbeat(error=str(error)[:300]); time.sleep(120)
        finally:
            self.job = None
            try:
                import torch
                torch.cuda.empty_cache()
            except ImportError:
                pass

    # ---- prepare: native frozen prefix for every admitted variant ---------------------
    def prepare(self, spec):
        from .data import NativeTokenizer, build_examples, load_families, render, split_digest, tokenize_example
        from .model import inspect_model
        from .native import build_native, extract_text, sha_file
        tranche = Path(spec['tranche'])
        families = load_families(tranche)
        examples = build_examples(families, spec.get('seed', 17))
        model = inspect_model(Path(self.config.model_path), self.config.base_sha256)
        if self.binary is None:
            self.binary = build_native(self.config, self.headers)
        shim = sha_file(self.binary)
        tokenizer = NativeTokenizer(self.config)
        records, rejected = [], []
        for number, example in enumerate(examples):
            if self.stopping():
                raise Interrupted()
            self.heartbeat(progress=f'prepare {number}/{len(examples)}')
            try:
                tokens = tokenize_example(example, tokenizer, self.config.sequence_length)
            except AdmissionError as error:
                rejected.append({'example_id': example.example_id, 'reason': str(error)}); continue
            _, full = render(example)
            root = self.root/'cache'/digest({'text': full, 'base': model.sha256, 'shim': shim})[:32]
            if not (root/'receipt.json').exists():
                if root.exists():
                    shutil.rmtree(root)
                extract_text(model, full, root, self.config, self.binary, ubatch=0, split=example.split)
                prune_reference_outputs(root)
            info = json.loads((root/'receipt.json').read_text())
            if info['tokens'] != tokens.tokens or info['split'] != example.split:
                rejected.append({'example_id': example.example_id, 'reason': 'native/tokenizer mismatch'}); continue
            records.append({**asdict(example), 'tokens': tokens.tokens, 'prompt_tokens': tokens.prompt_tokens,
                            'choice_row': tokens.choice_row, 'choice_token_ids': tokens.choice_token_ids,
                            'cache': str(root)})
        index = {'format_version': 1, 'tranche': str(tranche), 'tranche_sha256': sha_file(tranche),
                 'split_digest': split_digest(examples), 'base_sha256': model.sha256, 'shim_sha256': shim,
                 'examples': records, 'rejected': rejected, 'created_ms': now_ms()}
        out = self.root/'datasets'/(tranche.stem+'.json')
        atomic_json(out, index)
        return {'dataset': str(out), 'examples': len(records), 'rejected': len(rejected)}

    # ---- train -----------------------------------------------------------------------
    def train(self, spec):
        import torch
        from .head_loss import full_vocab_cross_entropy
        from .model import inspect_model
        from .native import validate_cache
        from .ptq import FrozenPTQLinear
        from .suffix import BonsaiSuffix
        run_id = spec['run_id']
        if not re.fullmatch(r'[a-z0-9-]{3,64}', run_id):
            raise AdmissionError('invalid run id')
        settings = {'targets': list(spec.get('targets', ['q', 'v'])), 'lr': float(spec.get('lr', 1e-4)),
                    'epochs': int(spec.get('epochs', 3)), 'accumulation': int(spec.get('accumulation', 8)),
                    'max_optimizer_steps': int(spec.get('max_optimizer_steps', 192)),
                    'max_minutes': float(spec.get('max_minutes', 60)), 'seed': int(spec.get('seed', 17)),
                    'eval_every': int(spec.get('eval_every', 4)), 'init_from': spec.get('init_from'),
                    'betas': [0.9, 0.999], 'eps': 1e-8, 'weight_decay': 0.0, 'grad_clip': 1.0,
                    'rank': self.config.rank, 'alpha': self.config.alpha}
        if not (0 < settings['lr'] <= 1e-3 and 1 <= settings['accumulation'] <= 64 and settings['epochs'] >= 1):
            raise AdmissionError('training settings outside reviewed bounds')
        examples = []
        for path in spec['datasets']:
            index = json.loads(Path(path).read_text())
            if index['base_sha256'] != BASE_SHA:
                raise AdmissionError('dataset built on a different base')
            examples += index['examples']
        train = [e for e in examples if e['split'] == 'train']
        dev = [e for e in examples if e['split'] == 'development']
        if not train or {e['family_id'] for e in train} & {e['family_id'] for e in dev}:
            raise AdmissionError('training set empty or a family crosses train/development')
        for e in examples:
            validate_cache(e['cache'], BASE_SHA, e['split'], e['tokens'], self.config.cache_cap_bytes)
        run_digest = digest({'settings': settings, 'base': BASE_SHA,
                             'data': sorted((e['example_id'], e['cache']) for e in examples)})
        run = self.root/'runs'/run_id
        run.mkdir(parents=True, exist_ok=True)
        model = inspect_model(Path(self.config.model_path), self.config.base_sha256)
        torch.manual_seed(settings['seed'])
        suffix = BonsaiSuffix(model, self.config, settings['targets']).cuda().make_resident('cuda')
        head = FrozenPTQLinear(model, 'output.weight', self.config.row_chunk).make_resident('cuda')
        names = [name for name, _ in suffix.named_parameters()]
        if not names or any(not re.fullmatch(r'[qkvo]_delta\.[ab]', n) for n in names):
            raise AdmissionError('only LoRA A/B tensors may be trainable')
        params = list(suffix.parameters())
        optimizer = torch.optim.AdamW(params, lr=settings['lr'], betas=tuple(settings['betas']),
                                      eps=settings['eps'], weight_decay=0.0)
        state = {'opt_step': 0, 'epoch': 0, 'position': 0, 'order': None, 'elapsed': 0.0,
                 'best': None, 'baseline': None, 'stop_reason': None}
        last = run/'last.pt'
        if last.exists():
            saved = torch.load(last, weights_only=True, map_location='cuda')
            if saved['run_digest'] != run_digest:
                raise AdmissionError('checkpoint belongs to different settings/data; refusing resume')
            suffix.load_state_dict(saved['model'], strict=False)
            optimizer.load_state_dict(saved['optimizer'])
            state = saved['state']
            torch.set_rng_state(saved['rng_cpu'].cpu()); torch.cuda.set_rng_state(saved['rng_cuda'].cpu())
        else:
            if settings['init_from']:
                initial = torch.load(settings['init_from'], weights_only=True, map_location='cuda')['model']
                if set(initial) - {k for k in suffix.state_dict() if '_delta.' in k}:
                    raise AdmissionError('init checkpoint has tensors outside this target set')
                suffix.load_state_dict(initial, strict=False)
            atomic_json(run/'spec.json', {'spec': spec, 'settings': settings, 'run_digest': run_digest,
                                          'train_examples': len(train), 'dev_examples': len(dev)})
            state['baseline'] = self.evaluate(suffix, head, dev, train)
            self.log(run, {'event': 'baseline', 'opt_step': 0, **state['baseline']})

        def lora_state():
            return {k: v.detach().clone() for k, v in suffix.state_dict().items() if '_delta.' in k}

        def checkpoint():
            atomic_torch_save({'run_digest': run_digest, 'model': lora_state(), 'optimizer': optimizer.state_dict(),
                               'state': state, 'rng_cpu': torch.get_rng_state(),
                               'rng_cuda': torch.cuda.get_rng_state()}, last)

        session_start, last_save = time.monotonic(), time.monotonic()
        base_elapsed = state['elapsed']
        torch.cuda.reset_peak_memory_stats()
        while state['epoch'] < settings['epochs'] and state['opt_step'] < settings['max_optimizer_steps']:
            if state['order'] is None:
                order = list(range(len(train)))
                random.Random(f'{settings["seed"]}:{state["epoch"]}').shuffle(order)
                state['order'], state['position'] = order, 0
            losses = []
            optimizer.zero_grad(set_to_none=True)
            while len(losses) < settings['accumulation'] and state['position'] < len(state['order']):
                example = train[state['order'][state['position']]]
                preflight(self.config, required_gpu_mib=512)
                loss = self.example_loss(suffix, head, example, full_vocab_cross_entropy)
                (loss/settings['accumulation']).backward()
                losses.append(float(loss.detach()))
                state['position'] += 1
            if len(losses) < settings['accumulation']:
                for p in params:
                    if p.grad is not None:
                        p.grad.mul_(settings['accumulation']/len(losses))
            norm = float(torch.nn.utils.clip_grad_norm_(params, settings['grad_clip'], error_if_nonfinite=True))
            optimizer.step()
            state['opt_step'] += 1
            if state['position'] >= len(state['order']):
                state['epoch'] += 1; state['order'] = None
            state['elapsed'] = base_elapsed+time.monotonic()-session_start
            self.log(run, {'event': 'step', 'opt_step': state['opt_step'], 'epoch': state['epoch'],
                           'train_loss': sum(losses)/len(losses), 'grad_norm': norm, 'elapsed': state['elapsed']})
            self.heartbeat(run=run_id, opt_step=state['opt_step'], train_loss=sum(losses)/len(losses))
            if state['opt_step'] % settings['eval_every'] == 0:
                metrics = self.evaluate(suffix, head, dev, train)
                self.log(run, {'event': 'eval', 'opt_step': state['opt_step'], **metrics})
                if self.better(metrics, state['best']):
                    state['best'] = {'opt_step': state['opt_step'], **metrics}
                    atomic_torch_save({'run_digest': run_digest, 'model': lora_state(),
                                       'settings': settings, 'metrics': metrics}, run/'best.pt')
                checkpoint(); last_save = time.monotonic()
            elif time.monotonic()-last_save > 300:
                checkpoint(); last_save = time.monotonic()
            if self.stopping():
                checkpoint(); raise Interrupted()
            while self.control() == 'pause':
                checkpoint(); self.state = 'paused'; self.heartbeat(run=run_id); time.sleep(20)
                if self.stopping():
                    raise Interrupted()
            self.state = 'running'
            if state['elapsed'] > settings['max_minutes']*60:
                state['stop_reason'] = 'max_minutes'; break
        state['stop_reason'] = state['stop_reason'] or 'schedule_complete'
        final = self.evaluate(suffix, head, dev, train)
        self.log(run, {'event': 'final', 'opt_step': state['opt_step'], **final})
        if self.better(final, state['best']):
            state['best'] = {'opt_step': state['opt_step'], **final}
            atomic_torch_save({'run_digest': run_digest, 'model': lora_state(), 'settings': settings,
                               'metrics': final}, run/'best.pt')
        checkpoint()
        receipt = {'format_version': 1, 'run_id': run_id, 'run_digest': run_digest, 'base_sha256': BASE_SHA,
                   'settings': settings, 'train_examples': len(train), 'dev_examples': len(dev),
                   'baseline': state['baseline'], 'final': final, 'best': state['best'],
                   'opt_steps': state['opt_step'], 'elapsed_seconds': state['elapsed'],
                   'stop_reason': state['stop_reason'],
                   'peak_cuda_allocated_bytes': torch.cuda.max_memory_allocated(),
                   'method': 'native-quantized-base LoRA on exact Bonsai2 9B block 31 (not canonical QLoRA)',
                   'data_review': 'owned-synthetic families authored by Claude, pending human review',
                   'claims': 'development metrics only; no sealed benchmark, no promotion'}
        atomic_json(run/'receipt.json', receipt)
        return {'receipt': str(run/'receipt.json'), 'best': state['best']}

    def log(self, run, record):
        with (run/'metrics.jsonl').open('a') as stream:
            stream.write(json.dumps({'ms': now_ms(), **record})+'\n')

    @staticmethod
    def better(metrics, best):
        if not metrics.get('dev_examples'):
            return best is None
        key = lambda m: (m['family_all_correct'], m['choice_accuracy'], -m['dev_loss'])
        return best is None or key(metrics) > key(best)

    def load_prefix(self, example):
        import torch
        rows = len(example['tokens'])
        values = np.fromfile(Path(example['cache'])/'prefix.f32', dtype='<f4')
        if values.size != rows*4096 or not np.isfinite(values).all():
            raise AdmissionError('prefix cache incomplete')
        return torch.from_numpy(values.reshape(rows, 4096)).cuda()

    def example_loss(self, suffix, head, example, loss_fn):
        import torch
        tokens, prompt = example['tokens'], example['prompt_tokens']
        labels = torch.tensor([tokens[i+1] if i+1 >= prompt else -100 for i in range(len(tokens)-1)]+[-100],
                              device='cuda', dtype=torch.int64)
        hidden = suffix(self.load_prefix(example))
        rows = labels != -100
        return loss_fn(hidden[rows], labels[rows], head, self.config.row_chunk)

    def evaluate(self, suffix, head, dev, train):
        """Development metrics from the model's own distribution, not formatting alone."""
        import torch
        from .head_loss import full_vocab_cross_entropy
        if not dev:
            return {'dev_examples': 0}
        rows, by_family = [], {}
        with torch.no_grad():
            for example in dev:
                tokens, prompt = example['tokens'], example['prompt_tokens']
                labels = torch.tensor([tokens[i+1] if i+1 >= prompt else -100 for i in range(len(tokens)-1)]+[-100],
                                      device='cuda', dtype=torch.int64)
                hidden = suffix(self.load_prefix(example))
                mask = labels != -100
                loss = float(full_vocab_cross_entropy(hidden[mask], labels[mask], head, self.config.row_chunk))
                record = {'variant': example['variant'], 'loss': loss, 'pred': None, 'p_correct': None}
                if example['choice_row'] is not None:
                    letters = list(example['option_letters'])
                    ids = [example['choice_token_ids'][letter] for letter in letters]
                    weights = torch.cat([head.chunk(i, hidden, 1) for i in ids])
                    logits = head.transform(hidden[example['choice_row']][None]) @ weights.T
                    probs = torch.softmax(logits[0].double(), -1)
                    pred = letters[int(probs.argmax())]
                    mapping = dict(example['shown_to_original'])
                    record.update(pred=pred, correct=pred == example['selected'],
                                  p_correct=float(probs[letters.index(example['selected'])]),
                                  original=mapping[pred], changes=example['counterfactual_changes'])
                rows.append(record)
                by_family.setdefault(example['family_id'], {})[example['variant']] = record
        scored = [r for r in rows if r['pred'] is not None]
        result = {'dev_examples': len(rows), 'dev_loss': sum(r['loss'] for r in rows)/len(rows),
                  'choice_scored': len(scored),
                  'choice_accuracy': sum(r['correct'] for r in scored)/max(1, len(scored)),
                  'mean_p_correct': sum(r['p_correct'] for r in scored)/max(1, len(scored))}
        for kind in ('clean', 'noise', 'cosmetic', 'counterfactual'):
            subset = [r for r in rows if r['variant'] == kind]
            scored_kind = [r for r in subset if r['pred'] is not None]
            result[f'loss_{kind}'] = sum(r['loss'] for r in subset)/max(1, len(subset))
            result[f'accuracy_{kind}'] = sum(r['correct'] for r in scored_kind)/max(1, len(scored_kind))
        families = [f for f in by_family.values() if all(v in f and f[v]['pred'] for v in ('clean', 'noise', 'cosmetic', 'counterfactual'))]
        n = max(1, len(families))
        result['families_scored'] = len(families)
        result['noise_invariance'] = sum(f['noise']['original'] == f['clean']['original'] for f in families)/n
        result['cosmetic_invariance'] = sum(f['cosmetic']['original'] == f['clean']['original'] for f in families)/n
        action = [f for f in families if f['counterfactual']['changes'] == 'action']
        result['counterfactual_flip_rate'] = (sum(f['counterfactual']['original'] != f['clean']['original'] for f in action)
                                              / max(1, len(action)))
        result['family_all_correct'] = sum(all(f[v]['correct'] for v in f) for f in families)/n
        return result
