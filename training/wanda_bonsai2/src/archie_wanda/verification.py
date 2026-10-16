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


def _native_run(binary, model, text, root, config, lora=None):
    import struct
    from .native import native_environment
    from .resources import run_guarded
    root=Path(root); root.mkdir(parents=True,exist_ok=False)
    raw=text.encode('utf-8')
    (root/'request.bin').write_bytes(b'ARWPRE01'+struct.pack('<IIII',1,0,config.sequence_length,len(raw))+raw)
    command=[str(binary),'--model',str(model.path),'--request',str(root/'request.bin'),
             '--out',str(root),'--runtime',config.runtime_dir]+(['--lora',str(lora)] if lora else [])
    with (root/'native.log').open('w') as log:
        process=run_guarded(command,config,native_environment(config),log,timeout=180)
    if process.returncode:
        raise ParityError('native adapter run failed; see '+str(root/'native.log'))
    return json.loads(process.stdout)


def _kl_rows(p_logits, q_logits):
    import torch
    log_p=torch.log_softmax(p_logits.double(),-1); log_q=torch.log_softmax(q_logits.double(),-1)
    return (log_p.exp()*(log_p-log_q)).sum(-1)


def verify_native(config, headers, candidate, checkpoint, cases, output):
    """Native Prism with the exported LoRA versus the Torch suffix with the same LoRA.

    Passes only if (1) the layer-30 capture is unchanged by a block-31 adapter, (2) native
    and Torch agree with the adapter on within native noise, and (3) the adapter moves the
    native distribution clearly more than that parity error (otherwise inconclusive).
    """
    import torch, uuid
    from .native import build_native, sha_file
    from .ptq import FrozenPTQLinear
    from .suffix import BonsaiSuffix
    from .native import LAYOUT_MAX_TOKEN_KL
    model=inspect_model(Path(config.model_path),config.base_sha256)
    binary=build_native(config,headers,'adapter_logits.cpp','archie-wanda-adapter')
    saved=torch.load(checkpoint,weights_only=True,map_location='cuda')
    targets=sorted({k.split('_delta.')[0] for k in saved['model']},key='qkvo'.index)
    suffix=BonsaiSuffix(model,config,targets).cuda().make_resident('cuda')
    suffix.load_state_dict(saved['model'],strict=False)
    head=FrozenPTQLinear(model,'output.weight',config.row_chunk).make_resident('cuda')
    texts=[]
    for line in Path(cases).read_text(encoding='utf-8').splitlines():
        case=json.loads(line); texts.append((case['id'],case['text']))
    work=Path(config.run_dir)/'verify'/uuid.uuid4().hex
    records=[]
    for case_id,text in texts:
        base=_native_run(binary,model,text,work/(case_id+'-base'),config)
        tuned=_native_run(binary,model,text,work/(case_id+'-lora'),config,candidate)
        if base['tokens']!=tuned['tokens']:
            raise ParityError('adapter changed tokenization')
        count=len(base['tokens'])
        load=lambda name,root,cols: torch.from_numpy(np.fromfile(root/name,dtype='<f4').reshape(count,cols))
        prefix_base,prefix_lora=load('prefix.f32',work/(case_id+'-base'),4096),load('prefix.f32',work/(case_id+'-lora'),4096)
        native_base=load('logits.f32',work/(case_id+'-base'),248320)
        native_lora=load('logits.f32',work/(case_id+'-lora'),248320)
        with torch.no_grad():
            torch_lora=head(suffix(prefix_base.cuda())).cpu()
        parity=_kl_rows(native_lora,torch_lora); effect=_kl_rows(native_base,native_lora)
        records.append({'id':case_id,'tokens':count,
                        'prefix_unchanged':bool(torch.equal(prefix_base,prefix_lora)),
                        'parity_max_kl':float(parity.max()),'parity_mean_kl':float(parity.mean()),
                        'adapter_effect_mean_kl':float(effect.mean()),'adapter_effect_max_kl':float(effect.max()),
                        'greedy_agreement_native_vs_torch':float((native_lora.argmax(-1)==torch_lora.argmax(-1)).double().mean())})
        shutil_rm(work/(case_id+'-base')); shutil_rm(work/(case_id+'-lora'))
    parity_ok=all(r['prefix_unchanged'] and r['parity_max_kl']<=max(1e-3,LAYOUT_MAX_TOKEN_KL) for r in records)
    effect=sum(r['adapter_effect_mean_kl'] for r in records)/len(records)
    error=sum(r['parity_mean_kl'] for r in records)/len(records)
    status='passed' if parity_ok and effect>=10*error else ('inconclusive_small_adapter_effect' if parity_ok else 'failed')
    result={'format_version':1,'status':status,'scope':'native_lora_candidate_vs_torch_suffix',
            'base_sha256':model.sha256,'candidate':str(candidate),'candidate_sha256':sha_file(candidate),
            'checkpoint':str(checkpoint),'targets':targets,'adapter_binary_sha256':sha_file(binary),
            'cases':records,'mean_adapter_effect_kl':effect,'mean_parity_kl':error,'promoted':False}
    atomic_json(output,result)
    if status=='failed':
        raise ParityError('native LoRA candidate does not match the trained suffix: '+json.dumps(records))
    return result


def shutil_rm(path):
    import shutil
    shutil.rmtree(path,ignore_errors=True)


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
