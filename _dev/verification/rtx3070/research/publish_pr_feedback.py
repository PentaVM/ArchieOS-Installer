"""Publish/update one owner-authorized PR38 research comment, without logging secrets."""
import argparse
import json
from pathlib import Path
import subprocess
import urllib.error
import urllib.request


def publish(body_file, receipt_file):
    origin=subprocess.run(['git','remote','get-url','origin'],check=True,capture_output=True,text=True).stdout.strip()
    if origin not in ('https://github.com/PentaVM/ArchieOS-Nvidia.git','git@github.com:PentaVM/ArchieOS-Nvidia.git'):
        raise ValueError('wrong repository origin')
    body=Path(body_file).read_text(encoding='utf-8')
    if not 0<len(body.encode('utf-8'))<=40000: raise ValueError('comment body outside bounds')
    result=subprocess.run(['git','-c','credential.interactive=never','credential','fill'],
        input='protocol=https\nhost=github.com\npath=PentaVM/ArchieOS-Nvidia.git\n\n',
        text=True,capture_output=True,check=True)
    fields=dict(line.split('=',1) for line in result.stdout.splitlines() if '=' in line)
    headers={'Authorization':'Bearer '+fields['password'],'Accept':'application/vnd.github+json',
             'Content-Type':'application/json','User-Agent':'ArchieOS-Wanda-Research'}
    del fields,result
    receipt=Path(receipt_file)
    old=json.loads(receipt.read_text()) if receipt.exists() else None
    endpoint='https://api.github.com/repos/PentaVM/ArchieOS-Nvidia/issues/38/comments'
    if old:
        if type(old.get('id')) is not int or old['id']<=0: raise ValueError('invalid existing comment identifier')
        endpoint='https://api.github.com/repos/PentaVM/ArchieOS-Nvidia/issues/comments/'+str(old['id'])
    request=urllib.request.Request(endpoint,data=json.dumps({'body':body}).encode('utf-8'),headers=headers,
                                   method='PATCH' if old else 'POST')
    with urllib.request.urlopen(request,timeout=20) as response:
        record=json.load(response)
    output={'id':record['id'],'url':record['html_url'],'issue':38,'body_file':str(body_file)}
    receipt.parent.mkdir(parents=True,exist_ok=True)
    temporary=receipt.with_suffix('.partial')
    temporary.write_text(json.dumps(output,indent=2)+'\n',encoding='utf-8'); temporary.replace(receipt)
    print(json.dumps(output))


if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--body',required=True); parser.add_argument('--receipt',required=True)
    args=parser.parse_args()
    try: publish(args.body,args.receipt)
    except urllib.error.HTTPError as error: raise SystemExit('GitHub comment request failed: HTTP '+str(error.code))
