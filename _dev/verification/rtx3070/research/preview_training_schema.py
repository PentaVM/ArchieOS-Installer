"""Tiny training-only API previews. Publishes schemas/statistics, never raw comments."""
from datetime import datetime,timezone
import hashlib
import json
from pathlib import Path
import sys
import urllib.parse
from fetch_source_metadata import fetch

root=Path(sys.argv[1]); root.mkdir(parents=True,exist_ok=True)
receipts=[]
for dataset,config in [('allenai/soda','default'),('google-research-datasets/go_emotions','simplified')]:
    record={'dataset':dataset,'split':'train','offset':0,'requested_rows':3}
    try:
        raw,_=fetch('https://huggingface.co/api/datasets/'+dataset)
        hub=json.loads(raw); record['hub_revision_at_preview']=hub['sha']
        record['card_license']=hub.get('cardData',{}).get('license')
        card_url='https://huggingface.co/datasets/'+dataset+'/raw/'+hub['sha']+'/README.md'
        card,final=fetch(card_url)
        file=dataset.replace('/','-')+'-card.md'; (root/file).write_bytes(card)
        record['card']={'url':final,'file':file,'sha256':hashlib.sha256(card).hexdigest()}
        url='https://datasets-server.huggingface.co/rows?'+urllib.parse.urlencode({
            'dataset':dataset,'config':config,'split':'train','offset':0,'length':3})
        raw,final=fetch(url); response=json.loads(raw); rows=[r['row'] for r in response['rows']]
        record.update(url=final,response_sha256=hashlib.sha256(raw).hexdigest(),
                      sampled_rows=len(rows),features=response['features'],raw_rows_retained=False,
                      viewer_is_pinned_to_hub_revision=False)
        # Dataset Viewer is a live view, not evidence that these bytes came from the pinned revision.
        record['sample_text_character_lengths']=[len(json.dumps(row,ensure_ascii=False)) for row in rows]
        if dataset=='allenai/soda':
            record['grouping_fields_present']={name:all(name in row for row in rows) for name in ['original_index','head','relation','tail']}
    except Exception as error:
        record['error']=type(error).__name__+': '+str(error)
    receipts.append(record)
result={'timestamp_utc':datetime.now(timezone.utc).isoformat(),'cuda_used':False,
        'scope':'training-only three-row schema preview, not representativeness or corpus admission',
        'previews':receipts}
(root/'training-preview.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
print(json.dumps(result,indent=2))
