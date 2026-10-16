"""Separate native extraction process: no Torch import or model side effects."""
from dataclasses import asdict
import hashlib
import json
import math
import os
import shutil
from pathlib import Path
import struct
import subprocess
import uuid
import numpy as np
from .contracts import BASE_SHA, PRISM_COMMIT, AdmissionError, ParityError
from .model import signature
from .resources import preflight, run_guarded

HEADER_SHA = {
    'include/llama.h': '50d9375b080df279b9a5d93df9e489f18785b0b9fa98a51045a7c637433e0d9c',
    'ggml/include/ggml.h': 'dc753f24e9dcf7b391e03ffa29c890e427b39f150d8f8473c08b7ecdf8f55856',
    'ggml/include/ggml-cpu.h': '316279e004cdeb8e6ef78599acb602bf79a8abdf897fed9fd1914808c1518c6e',
    'ggml/include/ggml-backend.h': '46d84cb998105f871240864fd0f55446939a2fe86c5c281afa63a010fb1f65a2',
    'ggml/include/ggml-opt.h': '3586de1bc8a934b5c72339e2b6937b0641e8f149b512231e666f67de0736eea2',
    'ggml/include/gguf.h': 'e56714aab702e5ce62ee587a409643c08f7e93e8fbb77f48ef7cc85075f96fa4',
    'ggml/include/ggml-alloc.h': '94e4cd069b9313b2ceb35dacec901981e0bb478d8bb31035b7126be091998c23',
}


def sha_file(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def validate_cache(root, expected_base, expected_split, expected_tokens, cap):
    root = Path(root).resolve()
    raw = (root/'receipt.json').read_bytes()
    if len(raw)>65536: raise AdmissionError('cache receipt too large')
    info = json.loads(raw)
    count = len(expected_tokens)
    if (info.get('format_version')!=1 or info.get('base_sha256')!=expected_base or
        info.get('split')!=expected_split or expected_split not in ('train','development','parity') or
        info.get('tokens')!=list(expected_tokens) or info.get('positions')!=list(range(count)) or
        info.get('sequence_ids')!=[0]*count or info.get('control_vectors')!='none' or info.get('capture_layer')!=30):
        raise AdmissionError('cache base/split/token/position/state mismatch')
    entries = info.get('files',{})
    if 'prefix.f32' not in entries or entries['prefix.f32']['shape']!=[count,info.get('hidden_size')]:
        raise AdmissionError('complete prefix rows required')
    total = 0
    for name, entry in entries.items():
        path = (root/name).resolve()
        if path.parent!=root or not path.is_file() or entry.get('dtype')!='float32':
            raise AdmissionError('invalid cache file')
        shape = entry.get('shape',[])
        if not shape or any(type(n) is not int or n<=0 for n in shape) or shape[0]!=count:
            raise AdmissionError('invalid cache dimensions')
        size = math.prod(shape)*4; total += size
        if total>cap or size!=path.stat().st_size or sha_file(path)!=entry.get('sha256'):
            raise AdmissionError('cache cap/digest/size mismatch')
        values = np.memmap(path,mode='r',dtype='<f4')
        for first in range(0,values.size,262144):
            if not np.isfinite(values[first:first+262144]).all():
                raise AdmissionError('nonfinite cache')
        del values
    return info


def native_environment(config):
    env = os.environ.copy()
    env['LD_LIBRARY_PATH'] = config.runtime_dir + ':' + config.runtime_dir + '/lib:/usr/lib/wsl/lib'
    return env


def build_native(config, headers, source_name='prefix_export.cpp', binary_name='archie-wanda-prefix'):
    preflight(config)
    headers = Path(headers)
    for name, expected in HEADER_SHA.items():
        if sha_file(headers/name)!=expected:
            raise AdmissionError('pinned native header digest mismatch: '+name)
    environment = native_environment(config)
    version = subprocess.run([config.runtime_dir+'/llama-server','--version'],env=environment,
                             check=True,capture_output=True,text=True,timeout=15)
    if PRISM_COMMIT[:9] not in version.stdout+version.stderr:
        raise AdmissionError('native runtime version mismatch')
    out = Path(config.run_dir)/'bin'; out.mkdir(parents=True,exist_ok=True)
    source = Path(__file__).resolve().parents[2]/'native'/source_name
    binary = out/binary_name
    command = ['g++','-std=c++17','-O2',str(source),'-I'+str(headers/'include'),
               '-I'+str(headers/'ggml/include'),'-L'+config.runtime_dir,'-lllama','-lggml','-lggml-base',
               '-Wl,-rpath,'+config.runtime_dir,'-o',str(binary)]
    result = subprocess.run(command,env=environment,capture_output=True,text=True,timeout=60)
    if result.returncode:
        raise AdmissionError('native shim compilation failed: '+result.stderr[-6000:])
    return binary


def extract_text(model, text, root, config, binary, *, ubatch=0, split='parity'):
    if model.sha256!=BASE_SHA or signature(model.path)!=model.signature or split not in ('train','development','parity'):
        raise AdmissionError('extraction requires unchanged approved base/admitted split')
    raw = text.encode('utf-8')
    if not 0<len(raw)<=32768 or type(ubatch) is not int or not 0<=ubatch<=512:
        raise AdmissionError('extraction text/batch outside bounds')
    resources = preflight(config,required_gpu_mib=3000)
    root = Path(root)
    root.mkdir(parents=True,exist_ok=False)
    request = root/'request.bin'
    worst_bytes = config.sequence_length*(248320+8192)*4
    cache_root = Path(config.run_dir)/'cache'
    existing_bytes = sum(p.stat().st_size for p in cache_root.rglob('*') if p.is_file()) if cache_root.exists() else 0
    if existing_bytes+worst_bytes>config.cache_cap_bytes or shutil.disk_usage(root).free<config.minimum_free_disk_bytes+worst_bytes:
        raise AdmissionError('cache/storage budget insufficient before extraction')
    request.write_bytes(b'ARWPRE01'+struct.pack('<IIII',1,ubatch,config.sequence_length,len(raw))+raw)
    command = [str(binary),'--model',str(model.path),'--request',str(request),
               '--out',str(root),'--runtime',config.runtime_dir]
    with (root/'native.log').open('w') as log:
        process = run_guarded(command,config,native_environment(config),log,timeout=180)
    if process.returncode:
        raise AdmissionError('native extraction failed; see '+str(root/'native.log'))
    result = json.loads(process.stdout)
    count = len(result['tokens'])
    if not 0<count<=config.sequence_length or result.get('rows')!=count or result.get('hidden_size')!=4096:
        raise AdmissionError('native extraction incomplete/outside sequence limit')
    shape_map = {'prefix.f32':[count,4096],'norm.f32':[count,4096],'logits.f32':[count,248320]}
    info = {'format_version':1,'base_sha256':model.sha256,'split':split,
            'tokens':result['tokens'],'positions':list(range(count)),'sequence_ids':[0]*count,
            'control_vectors':'none','capture_layer':30,'hidden_size':4096,
            'settings':result,'resources_before':asdict(resources),'runtime_commit':PRISM_COMMIT,
            'shim_sha256':sha_file(binary),'metadata_sha256':model.metadata_sha256,
            'files':{name:{'shape':shape,'dtype':'float32','sha256':sha_file(root/name)}
                     for name,shape in shape_map.items()}}
    (root/'receipt.json').write_text(json.dumps(info,indent=2)+'\n')
    if signature(model.path)!=model.signature:
        raise AdmissionError('base changed during native extraction')
    return validate_cache(root,model.sha256,split,result['tokens'],config.cache_cap_bytes)


def relative_rms(path_a, path_b):
    a = np.memmap(path_a,mode='r',dtype='<f4'); b = np.memmap(path_b,mode='r',dtype='<f4')
    if a.shape!=b.shape: raise ParityError('native comparison shape mismatch')
    error, scale, maximum = 0., 0., 0.
    for first in range(0,a.size,262144):
        left=a[first:first+262144].astype(np.float64); right=b[first:first+262144].astype(np.float64)
        delta=left-right
        error+=float(np.dot(delta,delta)); scale+=float(np.dot(left,left))
        maximum=max(maximum,float(np.abs(delta).max()))
    return {'relative_rms':math.sqrt(error/max(scale,1e-30)), 'max_abs':maximum}


def logit_agreement(path_a, path_b, vocabulary=248320):
    a = np.memmap(path_a,mode='r',dtype='<f4'); b = np.memmap(path_b,mode='r',dtype='<f4')
    if a.shape!=b.shape or a.size%vocabulary: raise ParityError('native logit shape mismatch')
    kls, agree = [], []
    for row in range(a.size//vocabulary):
        p=a[row*vocabulary:(row+1)*vocabulary].astype(np.float64)
        q=b[row*vocabulary:(row+1)*vocabulary].astype(np.float64)
        p-=p.max(); q-=q.max()
        log_p=p-np.log(np.exp(p).sum()); log_q=q-np.log(np.exp(q).sum())
        kls.append(float((np.exp(log_p)*(log_p-log_q)).sum())); agree.append(int(p.argmax()==q.argmax()))
    return {'max_token_kl':max(kls),'mean_token_kl':sum(kls)/len(kls),'greedy_agreement':sum(agree)/len(agree)}


# Measured 2026-10-16 on the exact base: a clean repeat with the same batch layout
# is bit-identical, while a different ubatch split changes CUDA kernel selection
# (activation quantization/accumulation order) by ~1% hidden RMS with KL ~1e-4.
# The capture contract is therefore exact repeatability; the layout difference is
# recorded as the native noise floor and bounded on next-token distributions.
# Greedy agreement is recorded, not gated: native itself flipped 1/35 near-tied
# argmaxes between layouts (KL 6.5e-4), so it measures ties rather than capture.
RESET_RELATIVE_RMS = 5e-4
LAYOUT_MAX_TOKEN_KL = 1e-3


def judge_native_comparisons(comparisons):
    reset = comparisons['fresh_reset']
    if any(reset[name]['relative_rms']>RESET_RELATIVE_RMS for name in ['prefix.f32','norm.f32','logits.f32']):
        raise ParityError('native clean repeat is not reproducible: '+json.dumps(reset))
    layout = comparisons['microbatch']['distribution']
    if layout['max_token_kl']>LAYOUT_MAX_TOKEN_KL:
        raise ParityError('native batch-layout noise exceeds bound: '+json.dumps(layout))


def verify_extraction(config, headers, cases, output):
    from .model import inspect_model
    model=inspect_model(Path(config.model_path),config.base_sha256)
    binary=build_native(config,headers)
    root=Path(config.run_dir)/'cache'/uuid.uuid4().hex
    results=[]
    for line in Path(cases).read_text().splitlines():
        case=json.loads(line)
        if set(case)!={'id','text'} or not case['id'].replace('-','').isalnum():
            raise AdmissionError('invalid public parity case')
        paths=[root/(case['id']+'-'+str(index)) for index in range(3)]
        receipts=[extract_text(model,case['text'],path,config,binary,ubatch=batch)
                  for path,batch in zip(paths,[0,4,0])]
        comparisons={}
        for index,label in [(1,'microbatch'),(2,'fresh_reset')]:
            if receipts[0]['tokens']!=receipts[index]['tokens']:
                raise ParityError('native token layout changed')
            comparisons[label]={name:relative_rms(paths[0]/name,paths[index]/name)
                                for name in ['prefix.f32','norm.f32','logits.f32']}
            comparisons[label]['distribution']=logit_agreement(paths[0]/'logits.f32',paths[index]/'logits.f32')
        judge_native_comparisons(comparisons)
        results.append({'id':case['id'],'tokens':len(receipts[0]['tokens']),
                        'paths':[str(p) for p in paths],'comparisons':comparisons})
    noise={'norm_relative_rms':max(r['comparisons']['microbatch']['norm.f32']['relative_rms'] for r in results),
           'max_token_kl':max(r['comparisons']['microbatch']['distribution']['max_token_kl'] for r in results)}
    receipt={'format_version':1,'scope':'real_native_full_row_capture_microbatch_reset',
             'base_sha256':model.sha256,'runtime_commit':PRISM_COMMIT,
             'shim_sha256':sha_file(binary),'cases':results,'native_layout_noise_floor':noise,
             'gates':{'reset_relative_rms':RESET_RELATIVE_RMS,'layout_max_token_kl':LAYOUT_MAX_TOKEN_KL,
                      'revision':'microbatch rms 5e-4 gate replaced after measured native layout noise; '
                                 'see native.py comment and ledger'},
             'full_model_backward':False}
    output=Path(output); output.parent.mkdir(parents=True,exist_ok=True)
    with output.open('x') as stream: json.dump(receipt,stream,indent=2)
    return receipt
