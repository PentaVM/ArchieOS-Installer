"""Read-only corpus/scoring audit. Standard library only; never invokes a model."""
import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def audit(repository):
    package=repository/'training/wanda_bonsai2'
    sys.path.insert(0,str(package/'src'))
    from archie_wanda.data import load_families, build_examples, render
    sources=sorted((package/'data/social').glob('tranche-*.json'))
    families=[family for source in sources for family in load_families(source)]
    examples=build_examples(families,17)
    groups={split:[e for e in examples if e.split==split] for split in ('train','development')}
    family_ids=[f['family_id'] for f in families]
    split_ids={split:{e.family_id for e in values} for split,values in groups.items()}
    assert len(family_ids)==len(set(family_ids)), 'duplicate family identifiers'
    assert not split_ids['train']&split_ids['development'], 'cross-split family identifiers'
    failures=[]
    for family in families:
        by_kind={e.variant:e for e in build_examples([family],17)}
        for kind in ('noise','cosmetic'):
            e=by_kind[kind]; clean=by_kind['clean']
            original=dict(e.shown_to_original)[e.selected]
            if original!=dict(clean.shown_to_original)[clean.selected]:
                failures.append({'family':family['family_id'],'variant':kind,'issue':'key_changed'})
    target_prefixes=[]
    for e in examples:
        target=json.loads(e.target)
        prefix=e.target[:e.target.index('"selected_option_id"')]
        target_prefixes.append('"consequences"' in prefix and '"relational_costs"' in prefix)
        prompt,full=render(e)
        assert full.startswith(prompt) and len(full)>len(prompt)
        assert set(dict(e.shown_to_original))==set(e.option_letters)
        assert len(target['alternatives'])==len(e.option_letters)
    summary={}
    for split,values in groups.items():
        original=[dict(e.shown_to_original)[e.selected] for e in values]
        def length_choice(example, longest):
            offered=example.user.split('Opciones:\n',1)[1].rsplit('\nDevuelve decision_v1.',1)[0]
            options=dict(line.split(': ',1) for line in offered.splitlines())
            return sorted(options,key=lambda letter:((-len(options[letter]) if longest else len(options[letter])),letter))[0]
        summary[split]={
            'families':len(split_ids[split]),'examples':len(values),
            'domains':dict(Counter(e.domain for e in values)),
            'selected_shown_letter':dict(Counter(e.selected for e in values)),
            'selected_author_original_letter':dict(Counter(original)),
            'random_option_expected_accuracy':sum(1/len(e.option_letters) for e in values)/len(values),
            'always_author_original_B_accuracy':sum(key=='B' for key in original)/len(values),
            'original_B_is_an_oracle_audit_not_a_deployable_baseline':True,
            'prompt_only_longest_option_accuracy':sum(length_choice(e,True)==e.selected for e in values)/len(values),
            'prompt_only_shortest_option_accuracy':sum(length_choice(e,False)==e.selected for e in values)/len(values),
            'prompt_only_always_A_accuracy':sum(e.selected=='A' for e in values)/len(values),
            'variants':dict(Counter(e.variant for e in values)),
            'reference_answer_character_lengths':{'min':min(len(e.target) for e in values),
                                                'max':max(len(e.target) for e in values)},
        }
    norm=lambda text:re.sub(r'\W+',' ',text.lower()).strip()
    exact_cross=[]
    for train in families:
        if train['split']!='train': continue
        for dev in families:
            if dev['split']=='development' and norm(train['situation'])==norm(dev['situation']):
                exact_cross.append([train['family_id'],dev['family_id']])
    assert 'torch' not in sys.modules, 'CPU audit must not import Torch'
    return {'format_version':1,'timestamp_utc':datetime.now(timezone.utc).isoformat(),
            'scope':'CPU data/metric audit, no model inference or training',
            'files':[{'path':str(p.relative_to(repository)),'sha256':sha(p)} for p in sources],
            'families':len(families),'strata':dict(Counter(f['stratum'] for f in families)),
            'provenance_review':dict(Counter(f['provenance'].get('review','missing') for f in families)),
            'counterfactual_change_kinds':dict(Counter(f['counterfactual']['changes'] for f in families)),
            'split_counts':summary,'variant_key_invariance_failures':failures,
            'exact_situation_cross_split_pairs':exact_cross,
            'gold_consequences_before_selected_key':sum(target_prefixes),
            'total_rendered_examples':len(examples),
            'teacher_forced_choice_is_blind_decision_evaluation':False,
            'contamination_from_pretraining_ruled_out':False,
            'model_benchmark_executed':False,'cuda_used':False}


if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--repository',required=True); parser.add_argument('--out',required=True)
    args=parser.parse_args(); result=audit(Path(args.repository).resolve())
    output=Path(args.out); output.parent.mkdir(parents=True,exist_ok=True)
    output.write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print(json.dumps(result,ensure_ascii=True,indent=2))
