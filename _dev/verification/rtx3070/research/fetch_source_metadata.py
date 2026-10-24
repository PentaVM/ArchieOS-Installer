"""Bounded public primary metadata fetch; no weights/corpus downloads or imports."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import urllib.parse
import urllib.request

REPOS={
    'fantom':('skywalker023/fantom',['README.md','LICENSE']),
    'casino':('kushalchawla/CaSiNo',['README.md','LICENSE']),
    'negotiator':('facebookresearch/end-to-end-negotiator',['README.md','LICENSE']),
    'empathetic':('facebookresearch/EmpatheticDialogues',['README.md','LICENSE']),
    'atomic2020':('allenai/comet-atomic-2020',['README.md','LICENSE']),
    'socialreasoningbench':('microsoft/social-reasoning-bench',['README.md','LICENSE']),
    'socialbench':('X-PLUG/SocialBench',['README.md','LICENSE']),
    'opentom':('seacowx/OpenToM',['README.md','LICENSE']),
}
LIMIT=262144


def fetch(url):
    request=urllib.request.Request(url,headers={'User-Agent':'ArchieOS-Wanda-CPU-Research/1.0'})
    with urllib.request.urlopen(request,timeout=20) as response:
        if urllib.parse.urlparse(response.url).scheme!='https': raise ValueError('non-HTTPS redirect')
        raw=response.read(LIMIT+1)
        if len(raw)>LIMIT: raise ValueError('metadata exceeds bounded fetch')
        return raw,response.url


def main(root):
    root.mkdir(parents=True,exist_ok=True)
    receipts=[]
    for name,(repo,files) in REPOS.items():
        try:
            raw,_=fetch('https://api.github.com/repos/'+repo+'/commits/HEAD')
            revision=json.loads(raw)['sha']
        except Exception as error:
            receipts.append({'source':name,'error':type(error).__name__+': '+str(error)}); continue
        for file in files:
            url='https://raw.githubusercontent.com/'+repo+'/'+revision+'/'+file
            try:
                raw,final=fetch(url); destination=root/(name+'-'+file.replace('/','-'))
                destination.write_bytes(raw)
                receipts.append({'source':name,'repo':repo,'revision':revision,'file':file,
                                 'url':final,'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest()})
            except Exception as error:
                receipts.append({'source':name,'repo':repo,'revision':revision,'file':file,'url':url,
                                 'error':type(error).__name__+': '+str(error)})
    # Only metadata is downloaded here. No benchmark/test examples are exposed to the trainer.
    for dataset in ['allenai/soda','google-research-datasets/go_emotions']:
        encoded=urllib.parse.urlencode({'dataset':dataset})
        for endpoint in ['splits','size']:
            url='https://datasets-server.huggingface.co/'+endpoint+'?'+encoded
            try:
                raw,final=fetch(url); parsed=json.loads(raw)
                file=dataset.replace('/','-')+'-'+endpoint+'.json'
                (root/file).write_bytes(raw)
                receipts.append({'source':dataset,'url':final,'file':file,'bytes':len(raw),
                                 'sha256':hashlib.sha256(raw).hexdigest()})
            except Exception as error:
                receipts.append({'source':dataset,'url':url,'error':type(error).__name__+': '+str(error)})
    result={'format_version':1,'timestamp_utc':datetime.now(timezone.utc).isoformat(),
            'scope':'public metadata only; no corpus admission or model inference',
            'cuda_used':False,'per_request_byte_cap':LIMIT,'receipts':receipts}
    (root/'receipts.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(result,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--out',required=True)
    main(Path(parser.parse_args().out))
