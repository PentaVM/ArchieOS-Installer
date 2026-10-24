"""Authorized own two-update experiment; no old queues or controls are modified."""
import argparse
from dataclasses import asdict
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO / 'training/wanda_bonsai2/src'))
from archie_wanda.contracts import TrainConfig, digest
from archie_wanda.native import sha_file
from archie_wanda.resources import preflight

OUT = Path(__file__).resolve().parent
SOURCE = Path('/mnt/c/Users/Penta/.codex/worktrees/wanda-bonsai9b/ArchieOS/_dev/verification/rtx3070/research/round2/curriculum-003-compact-candidate.json')
SOURCE_SHA = 'a6eff2ddebbe2755c4937a3da9f4aea1737738d49496b9070fc286edeac9202f'
FAMILY = 'r2c-cabina-interprete'
HEADERS = '/mnt/c/Users/Penta/Archie-RTX3070-Handoff/local-verification/prism-b10777-source'


def read_live_worker():
    state=json.loads(Path('/home/archie/wanda/run/worker.json').read_text())
    pid=state.get('pid')
    boot=Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    if type(pid) is not int or state.get('boot_id')!=boot:
        raise RuntimeError('worker PID/boot identity is invalid')
    command=Path(f'/proc/{pid}/cmdline').read_bytes().replace(b'\0',b' ').decode()
    if '-m archie_wanda.cli worker ' not in command:
        raise RuntimeError('worker PID is no longer the expected trainer process')
    return state


class OwnedHold:
    """Documented sharing handshake; the original worker keeps its own lock."""
    def __init__(self, hold, own_lock, inspect_worker=read_live_worker):
        self.hold,self.own_lock,self.inspect_worker=Path(hold),Path(own_lock),inspect_worker
        self.lock=None; self.identity=None; self.contents=None
        self.child=None

    def __enter__(self):
        self.lock=self.own_lock.open('a')
        try:
            fcntl.flock(self.lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.expected=self.inspect_worker()
            self.contents=json.dumps({'owner':'codex-round2-semantic','pid':os.getpid(),
                                      'token':uuid.uuid4().hex}).encode()
            descriptor=os.open(self.hold,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
            with os.fdopen(descriptor,'wb') as stream:
                self.identity=(os.fstat(stream.fileno()).st_dev,os.fstat(stream.fileno()).st_ino)
                stream.write(self.contents); stream.flush(); os.fsync(stream.fileno())
            return self
        except BaseException:
            self.lock.close(); self.lock=None
            raise

    def assert_paused(self):
        current=self.inspect_worker()
        if ((current.get('pid'),current.get('boot_id')) !=
                (self.expected.get('pid'),self.expected.get('boot_id')) or current.get('state')!='paused'):
            raise RuntimeError('coordinated worker identity/state changed or is not paused')
        return current

    def wait_paused(self, deadline):
        while time.monotonic()<deadline:
            current=self.inspect_worker()
            if ((current.get('pid'),current.get('boot_id')) !=
                    (self.expected.get('pid'),self.expected.get('boot_id'))):
                raise RuntimeError('worker identity changed while waiting for hold')
            if current.get('state')=='paused': return current
            time.sleep(.25)
        raise RuntimeError('worker did not pause within the hold deadline')

    def __exit__(self, *error):
        try:
            # Shutdown belongs INSIDE the hold lifetime. Failure to reap keeps the hold.
            if self.child is not None and self.child.poll() is None:
                os.killpg(self.child.pid,signal.SIGTERM)
                try: self.child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(self.child.pid,signal.SIGKILL)
                    self.child.wait(timeout=3)
            stat=self.hold.stat()
            if (stat.st_dev,stat.st_ino)!=self.identity or self.hold.read_bytes()!=self.contents:
                raise RuntimeError('hold ownership changed; refusing to remove it')
            self.hold.unlink()
        finally:
            if self.lock: self.lock.close()


def require_no_cuda_work():
    result=subprocess.run(['/usr/lib/wsl/lib/nvidia-smi','--query-compute-apps=pid',
                           '--format=csv,noheader,nounits'],capture_output=True,text=True,check=True,timeout=3)
    if result.stdout.strip():
        raise RuntimeError('CUDA compute process exists before the owned phase; no GPU job admitted')


def config_for(root):
    return TrainConfig(model_path='/home/archie/wanda/models/Ternary-Bonsai-2-9B-PTQ1_0.gguf',
                       run_dir=str(root), sequence_length=512, host_system_path='/mnt/c')


def save(path, value):
    from archie_wanda.verification import atomic_json
    atomic_json(path, value)


def prepare(root, config):
    from archie_wanda.data import NativeTokenizer, build_examples, load_families, render, split_digest, tokenize_example
    from archie_wanda.model import inspect_model
    from archie_wanda.native import build_native, extract_text
    from archie_wanda.worker import prune_reference_outputs
    if sha_file(SOURCE) != SOURCE_SHA:
        raise RuntimeError('admitted compact candidate bytes changed')
    families = load_families(SOURCE)
    chosen = [f for f in families if f['family_id'] == FAMILY]
    if len(chosen) != 1 or chosen[0]['domain'] != 'work' or chosen[0]['split'] != 'train':
        raise RuntimeError('only the selected complete work/train family is admitted')
    subset = root / 'cabina-subset.json'
    save(subset, chosen)
    examples = build_examples(chosen, 17)
    if len(examples) != 4:
        raise RuntimeError('all four family variants are required')
    model = inspect_model(Path(config.model_path), config.base_sha256)
    binary = build_native(config, HEADERS)
    shim = sha_file(binary)
    tokenizer = NativeTokenizer(config)
    records = []
    for example in examples:
        tokenized = tokenize_example(example, tokenizer, 512)
        count = len(tokenized.tokens)
        if count % 64 in (1,2,3,4):
            raise RuntimeError('strict preparation would enter a short Q8 remainder')
        _, full = render(example)
        cache = root / 'cache' / digest({'text': full, 'base': model.sha256,
                                       'shim': shim, 'precision_profile': 'strict-f32'})[:32]
        extract_text(model, full, cache, config, binary, ubatch=64, split='train', precision_profile='strict-f32')
        prune_reference_outputs(cache)
        records.append({**asdict(example), 'tokens': tokenized.tokens, 'prompt_tokens': tokenized.prompt_tokens,
                        'choice_row': tokenized.choice_row, 'choice_token_ids': tokenized.choice_token_ids,
                        'cache': str(cache)})
        print(json.dumps({'event': 'fresh_cache_complete', 'example_id': example.example_id,
                          'tokens': count, 'ubatch': 64}), flush=True)
    if sha_file(SOURCE) != SOURCE_SHA:
        raise RuntimeError('candidate changed during preparation')
    index = {'format_version':1, 'tranche':str(subset), 'tranche_sha256':sha_file(subset),
             'source_parent':str(SOURCE), 'source_parent_sha256':SOURCE_SHA,
             'selected_family_ids':[FAMILY], 'excluded_family_ids':[f['family_id'] for f in families if f['family_id']!=FAMILY],
             'selection':'complete work family fits512; no model outputs used',
             'precision_profile':'strict-f32', 'split_digest':split_digest(examples),
             'base_sha256':model.sha256, 'metadata_sha256':model.metadata_sha256,
             'shim_sha256':shim, 'examples':records, 'rejected':[], 'created_ms':int(time.time()*1000)}
    save(root / 'dataset.json', index)
    save(OUT / 'own-preparation.json', {k:v for k,v in index.items() if k!='examples'} | {
        'dataset':str(root/'dataset.json'), 'dataset_sha256':sha_file(root/'dataset.json'),
        'full_target_token_counts':{e['example_id']:len(e['tokens']) for e in records},
        'domain':'work', 'development_examples':0, 'semantic_quality_measured':False,
        'fresh_prefix_receipts':{e['example_id']:sha_file(Path(e['cache'])/'receipt.json') for e in records}})


def train(root, config):
    # Parent sets both process-only overrides before this fresh child imports Torch.
    import archie_wanda.worker as module
    spec = {'type':'train', 'run_id':'cabina-qv-s17-two-step', 'datasets':[str(root/'dataset.json')],
            'targets':['q','v'], 'lr':1e-4, 'accumulation':1, 'epochs':1,
            'max_optimizer_steps':2, 'eval_every':1, 'max_minutes':3, 'seed':17,
            'precision_profile':'strict-f32', 'extraction_receipt':str(OUT/'strict-extraction.json'),
            'parity_receipt':str(OUT/'strict-suffix.json'), 'backward_receipt':str(OUT/'strict-backward.json')}
    save(root/'train-spec.json', spec)
    original_save = module.atomic_torch_save
    def per_step_save(obj, path):
        original_save(obj, path)
        if Path(path).name == 'last.pt' and obj.get('state',{}).get('opt_step'):
            step = obj['state']['opt_step']
            snapshot = Path(path).with_name(f'step-{step:06}.pt')
            if not snapshot.exists():
                original_save(obj, snapshot)
    module.atomic_torch_save = per_step_save
    worker = module.Worker(config, root, HEADERS, json.loads((root/'deadline.json').read_text())['deadline_unix'])
    worker.acquire()  # own new root only; the documented hold keeps Claude paused
    result = worker.train(spec)
    receipt = json.loads(Path(result['receipt']).read_text())
    if receipt['opt_steps'] != 2 or receipt['stop_reason'] != 'schedule_complete':
        raise RuntimeError('two optimizer updates did not complete')
    save(OUT/'own-training.json', receipt | {'experiment_source_sha256':SOURCE_SHA,
        'family_ids':[FAMILY], 'domain':'work', 'experimental':True, 'pending_human_review':True,
        'semantic_quality_measured':False, 'normal_q8_serving_parity':'not_run',
        'run_root':str(root), 'receipt_sha256':sha_file(result['receipt']),
        'checkpoint_sha256':{p.name:sha_file(p) for p in sorted(Path(result['receipt']).parent.glob('*.pt'))}})


def export(root):
    import torch
    from archie_wanda.export import export_candidate
    run = root/'runs/cabina-qv-s17-two-step'
    before = torch.load(run/'step-000001.pt',weights_only=True,map_location='cpu')
    final = torch.load(run/'last.pt',weights_only=True,map_location='cpu')
    if final['state']['opt_step'] != 2 or final['run_digest'] != before['run_digest']:
        raise RuntimeError('CPU export checkpoint step/identity mismatch')
    changes = {name:float((tensor-before['model'][name]).norm()) for name,tensor in final['model'].items()}
    if any(not torch.isfinite(value).all().item() for value in final['model'].values()) or not all(v>0 for v in changes.values()):
        raise RuntimeError('real step1-to-step2 A/B updates are missing or nonfinite')
    candidate = root/'cabina-experimental-unpromoted.gguf'
    exported = export_candidate(run/'last.pt',candidate)
    index = json.loads((root/'dataset.json').read_text())
    order, position = final['state']['order'], final['state']['position']
    used = [index['examples'][i]['example_id'] for i in order[:position]]
    manifest = exported | {'checkpoint_sha256':sha_file(run/'last.pt'), 'optimizer_steps':2,
        'admission':final['admission'], 'settings':final['settings'],
        'source_parent_sha256':SOURCE_SHA, 'subset_sha256':index['tranche_sha256'],
        'dataset_sha256':sha_file(root/'dataset.json'), 'metadata_sha256':index['metadata_sha256'],
        'selected_family_ids':[FAMILY], 'domain':'work', 'examples_used_in_updates':used,
        'step1_to_step2_lora_delta_norms':changes, 'experimental':True, 'pending_human_review':True,
        'promoted':False, 'native_reload':'not_run', 'normal_q8_serving_parity':'not_run',
        'prompt_only_generation_quality':'not_run', 'semantic_quality_measured':False,
        'export_process_cuda_visible_devices':os.environ.get('CUDA_VISIBLE_DEVICES')}
    save(root/'candidate-manifest.json', manifest)
    save(OUT/'own-export.json', manifest)
    print(json.dumps({'event':'cpu_export_complete','candidate':str(candidate),'sha256':exported['sha256'],
                      'optimizer_steps':2,'examples_used':used,'lora_delta_norms':changes}),flush=True)


def sample(start):
    raw = subprocess.run(['/usr/lib/wsl/lib/nvidia-smi','--query-gpu=temperature.gpu,memory.used,power.draw',
                          '--format=csv,noheader,nounits'],capture_output=True,text=True,check=True,timeout=3)
    lines = raw.stdout.strip().splitlines()
    if len(lines)!=1: raise RuntimeError('single GPU telemetry required')
    temp,memory,power = map(float,lines[0].split(','))
    return {'elapsed':time.monotonic()-start,'temperature_c':temp,'used_gpu_mib':memory,'power_w':power}


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--root',required=True)
    parser.add_argument('--phase',choices=('guard','prepare','train','export'),default='guard')
    args=parser.parse_args()
    root=Path(args.root); config=config_for(root)
    if args.phase=='prepare': return prepare(root,config)
    if args.phase=='train': return train(root,config)
    if args.phase=='export': return export(root)
    started=time.monotonic(); started_unix=time.time()
    root.mkdir(parents=True,exist_ok=False)
    save(root/'config.json',asdict(config))
    save(root/'deadline.json',{'started_unix':started_unix,'deadline_unix':started_unix+180})
    receipt={'format_version':1,'scope':'own_semantic_two_update_experiment', 'started_unix':started_unix,
        'deadline_seconds':180,'start_below_c':60,'stop_at_c':70,'gpu_ceiling_mib':7168,
        'script_sha256':sha_file(__file__),'source_parent_sha256':SOURCE_SHA,
        'phases':[],'samples':[],'semantic_quality_measured':False,'promoted':False}
    owned=None
    try:
        with OwnedHold('/home/archie/wanda/run/hold','/home/archie/wanda/round2-codex-gpu.lock') as hold:
            receipt['coordination_protocol']='owned hold file; original worker retains its lock'
            receipt['original_worker_before_hold']=hold.expected
            receipt['original_worker_paused']=hold.wait_paused(min(started+180,time.monotonic()+45))
            initial=sample(started); receipt['samples'].append(initial)
            if initial['temperature_c']>=60: raise RuntimeError('GPU too warm at precheck')
            preflight(config)
            for phase in ('prepare','train','export'):
                hold.assert_paused()
                require_no_cuda_work()
                env=os.environ.copy()
                env['NVIDIA_TF32_OVERRIDE']='0'; env['TORCH_ALLOW_TF32_CUBLAS_OVERRIDE']='0'
                if phase=='export': env['CUDA_VISIBLE_DEVICES']='-1'
                with (root/(phase+'.log')).open('w') as log:
                    command=[sys.executable,__file__,'--root',str(root),'--phase',phase]
                    owned=subprocess.Popen(command,stdout=log,stderr=log,env=env,start_new_session=True)
                    hold.child=owned
                    record={'phase':phase,'pid':owned.pid,'command':command,'started_elapsed':time.monotonic()-started}
                    receipt['phases'].append(record)
                    print(json.dumps({'event':'owned_phase_started',**record}),flush=True)
                    while owned.poll() is None:
                        hold.assert_paused()
                        reading=sample(started); receipt['samples'].append(reading)
                        preflight(config)
                        if reading['temperature_c']>=70 or reading['used_gpu_mib']>7168:
                            raise RuntimeError('owned experiment exceeded thermal/memory threshold')
                        if time.monotonic()-started>=180: raise RuntimeError('whole experiment deadline exceeded')
                        time.sleep(.25)
                    record['exit_code']=owned.returncode; record['finished_elapsed']=time.monotonic()-started
                    if owned.returncode: raise RuntimeError('owned phase failed: '+phase+'; see '+str(root/(phase+'.log')))
                    owned=None; hold.child=None
            receipt['status']='passed_two_updates_exported_unpromoted'
    except BaseException as error:
        receipt['status']='partial_or_failed'; receipt['error']=str(error)
        raise
    finally:
        receipt['seconds']=time.monotonic()-started
        receipt['own_hold_removed']=not Path('/home/archie/wanda/run/hold').exists()
        receipt['max_temperature_c']=max(x['temperature_c'] for x in receipt['samples']) if receipt['samples'] else None
        receipt['max_gpu_mib']=max(x['used_gpu_mib'] for x in receipt['samples']) if receipt['samples'] else None
        save(OUT/'own-guard.json',receipt); save(root/'guard.json',receipt)
    print(json.dumps({k:receipt[k] for k in ('status','seconds','max_temperature_c','max_gpu_mib')}),flush=True)


if __name__=='__main__': main()
