"""Independent CPU shape/invariance/baseline audit; never invokes a model."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import sys


def main(candidate, repository, output):
    sys.path.insert(0,str(repository/'training/wanda_bonsai2/src'))
    from archie_wanda.data import load_families,build_examples,render
    raw=candidate.read_bytes()
    families=load_families(candidate); examples=build_examples(families,17)
    assert families and all(f['split']=='train' for f in families)
    assert all(f['provenance']['review']=='pending-human' for f in families)
    assert len({f['family_id'] for f in families})==len(families)
    for family in families:
        variants={e.variant:e for e in build_examples([family],17)}
        original=lambda e:dict(e.shown_to_original)[e.selected]
        assert original(variants['clean'])==original(variants['noise'])==original(variants['cosmetic'])
        for example in variants.values():
            prompt,full=render(example)
            assert full.startswith(prompt) and json.loads(example.target)['selected_option_id']==example.selected
    counts={}
    for kind in ['all','clean','noise','cosmetic','counterfactual']:
        rows=[e for e in examples if kind=='all' or e.variant==kind]
        scores=Counter(longest=0,shortest=0,always_A=0)
        for example in rows:
            offered=example.user.split('Opciones:\n',1)[1].rsplit('\nDevuelve decision_v1.',1)[0]
            options=dict(line.split(': ',1) for line in offered.splitlines())
            longest=sorted(options,key=lambda k:(-len(options[k]),k))[0]
            shortest=sorted(options,key=lambda k:(len(options[k]),k))[0]
            scores['longest']+=longest==example.selected
            scores['shortest']+=shortest==example.selected
            scores['always_A']+=example.selected=='A'
        counts[kind]={'n':len(rows),'correct':dict(scores)}
    assert 'torch' not in sys.modules
    result={'candidate_sha256':hashlib.sha256(raw).hexdigest(),
            'scope':'root CPU schema/invariance/baseline checks, no model inference or semantic quality proof',
            'families':len(families),'examples':len(examples),'domains':dict(Counter(f['domain'] for f in families)),
            'family_ids':sorted(f['family_id'] for f in families),'baselines':counts,
            'human_review':'pending','cuda_used':False}
    output.write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(result))


if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--candidate',type=Path,required=True)
    parser.add_argument('--repository',type=Path,required=True); parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args(); main(args.candidate,args.repository,args.out)
