"""Real native-reference suffix/head gates, separate from native CUDA loading."""
from dataclasses import asdict
import json
from pathlib import Path
import time
import numpy as np
from .contracts import AdmissionError, ParityError
from .model import inspect_model
from .native import validate_cache, sha_file
from .resources import preflight


def atomic_json(path, record):
    path=Path(path); path.parent.mkdir(parents=True,exist_ok=True)
    temporary=path.with_name(path.name+'.partial')
    with temporary.open('w') as stream:
        json.dump(record,stream,indent=2,allow_nan=False); stream.write('\n')
        stream.flush()
        import os
        os.fsync(stream.fileno())
    temporary.replace(path)


def load_cache(config, model, root):
    root=Path(root)
    raw=json.loads((root/'receipt.json').read_text())
    info=validate_cache(root,model.sha256,raw['split'],raw['tokens'],config.cache_cap_bytes)
    if (info.get('runtime_commit') is None or info['hidden_size']!=4096 or
        info['files'].get('norm.f32',{}).get('shape')!=[len(info['tokens']),4096] or
        info['files'].get('logits.f32',{}).get('shape')!=[len(info['tokens']),248320]):
        raise AdmissionError('missing complete native reference outputs')
    return info


def suffix_criterion(rms, kl, floor):
    """Strict predeclared tolerance, or no worse than native's own measured layout noise.

    Native CUDA is deterministic per batch layout but differs ~1% across layouts
    (see native.py); exact FP32 Torch math cannot be held tighter than native is
    to itself. Both criteria still require the next-token KL bound.
    """
    if rms<=5e-4 and kl<=1e-3:
        return 'strict'
    if rms<=floor['norm_relative_rms'] and kl<=max(1e-3,floor['max_token_kl']):
        return 'within_native_layout_noise'
    return None


def verify_suffix(config, extraction, output):
    import torch
    from .suffix import BonsaiSuffix
    from .ptq import FrozenPTQLinear
    before=preflight(config,required_gpu_mib=1500)
    native=json.loads(Path(extraction).read_text())
    if native.get('base_sha256')!=config.base_sha256 or len(native.get('cases',[]))<3:
        raise AdmissionError('complete three-case native extraction gate required')
    floor=native.get('native_layout_noise_floor')
    if not floor:
        raise AdmissionError('extraction receipt lacks measured native noise floor')
    model=inspect_model(Path(config.model_path),config.base_sha256)
    torch.manual_seed(config.seed); torch.cuda.reset_peak_memory_stats()
    suffix=BonsaiSuffix(model,config).cuda().make_resident('cuda')
    head=FrozenPTQLinear(model,'output.weight',config.row_chunk).make_resident('cuda')
    cases=[]; start=time.monotonic()
    with torch.no_grad():
        for case in native['cases']:
            preflight(config)
            root=Path(case['paths'][0]); info=load_cache(config,model,root); count=len(info['tokens'])
            prefix=torch.from_numpy(np.fromfile(root/'prefix.f32',dtype='<f4').reshape(count,4096)).cuda()
            actual=suffix(prefix)
            expected=torch.from_numpy(np.fromfile(root/'norm.f32',dtype='<f4').reshape(count,4096)).cuda()
            rms=float(((actual.double()-expected.double()).square().sum()/expected.double().square().sum()).sqrt())
            logits=head(actual).cpu().double()
            reference=torch.from_numpy(np.fromfile(root/'logits.f32',dtype='<f4').reshape(count,248320)).double()
            log_p=torch.log_softmax(reference,dim=-1); log_q=torch.log_softmax(logits,dim=-1)
            kl=float((log_p.exp()*(log_p-log_q)).sum(-1).max())
            record={'id':case['id'],'cache':str(root),'cache_sha256':sha_file(root/'receipt.json'),
                    'tokens':count,'normalized_relative_rms':rms,'maximum_token_kl':kl,
                    'greedy_agreement':float((logits.argmax(-1)==reference.argmax(-1)).double().mean())}
            record['criterion']=suffix_criterion(rms,kl,floor)
            cases.append(record)
            if record['criterion'] is None:
                failed={'format_version':1,'status':'failed','scope':'real_zero_adapter_suffix',
                        'base_sha256':model.sha256,'cases':cases}
                atomic_json(output,failed)
                raise ParityError('real native suffix/head tolerance exceeded: '+json.dumps(record))
            del logits,reference,log_p,log_q,prefix,actual,expected
    result={'format_version':1,'status':'passed','scope':'real_zero_adapter_suffix',
            'base_sha256':model.sha256,'extraction_sha256':sha_file(extraction),'cases':cases,
            'seconds':time.monotonic()-start,'peak_cuda_allocated_bytes':torch.cuda.max_memory_allocated(),
            'resources_before':asdict(before),'nonzero_native_adapter_verified':False,
            'semantic_training':False}
    atomic_json(output,result); return result


def verify_loss(config, parity, output):
    import torch
    from .suffix import BonsaiSuffix
    from .ptq import FrozenPTQLinear
    from .head_loss import full_vocab_cross_entropy
    before=preflight(config,required_gpu_mib=1500)
    receipt=json.loads(Path(parity).read_text())
    if (receipt.get('status')!='passed' or receipt.get('scope')!='real_zero_adapter_suffix' or
        receipt.get('base_sha256')!=config.base_sha256):
        raise AdmissionError('passed exact-base native suffix gate required')
    model=inspect_model(Path(config.model_path),config.base_sha256)
    root=Path(receipt['cases'][-1]['cache']); info=load_cache(config,model,root)
    if sha_file(root/'receipt.json')!=receipt['cases'][-1]['cache_sha256']:
        raise AdmissionError('parity cache changed')
    torch.manual_seed(config.seed); torch.cuda.reset_peak_memory_stats()
    suffix=BonsaiSuffix(model,config).cuda().make_resident('cuda')
    head=FrozenPTQLinear(model,'output.weight',config.row_chunk).make_resident('cuda')
    prefix=torch.from_numpy(np.fromfile(root/'prefix.f32',dtype='<f4').reshape(-1,4096)).cuda()
    labels=torch.tensor(info['tokens'][1:],device='cuda',dtype=torch.int64)
    start=time.monotonic(); optimizer=torch.optim.AdamW(suffix.parameters(),lr=1e-4,weight_decay=0.)
    records=[]
    for step in range(2):
        preflight(config); optimizer.zero_grad(set_to_none=True)
        value=suffix(prefix)[:-1]
        loss=full_vocab_cross_entropy(value,labels,head,config.row_chunk)
        loss.backward()
        gradients={name:float(p.grad.norm()) if p.grad is not None else None for name,p in suffix.named_parameters()}
        if any(p.grad is None or not torch.isfinite(p.grad).all().item() for p in suffix.parameters()):
            raise ParityError('missing/nonfinite full-path LoRA gradients')
        if step==1 and any(value is None or value<=0 for value in gradients.values()):
            raise ParityError('second-step A/B gradients must all be nonzero')
        torch.nn.utils.clip_grad_norm_(suffix.parameters(),1.,error_if_nonfinite=True)
        optimizer.step(); torch.cuda.synchronize()
        records.append({'step':step+1,'loss':float(loss.detach()),'gradient_norms':gradients})
    result={'format_version':1,'status':'passed','scope':'real_suffix_full_vocabulary_backward',
            'base_sha256':model.sha256,'parity_sha256':sha_file(parity),'tokens':len(info['tokens']),
            'vocabulary':head.rows,'steps':records,'seconds':time.monotonic()-start,
            'peak_cuda_allocated_bytes':torch.cuda.max_memory_allocated(),'resources_before':asdict(before),
            'semantic_training':False,'note':'Mechanical public parity text; no candidate promotion.'}
    atomic_json(output,result); return result
