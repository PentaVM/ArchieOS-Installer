import copy
import json
from pathlib import Path
import tempfile
import unittest
from archie_wanda.contracts import AdmissionError
from archie_wanda.data import (LETTERS, TokenExample, build_examples, expand_compact, load_families,
                               render, variant)

DATA = Path(__file__).resolve().parents[1]/'data'/'social'
TRANCHES = sorted(DATA.glob('tranche-*.json'))


def family():
    return copy.deepcopy(load_families(TRANCHES[0])[0])


class DataTests(unittest.TestCase):
    def test_all_admitted_tranches_render_every_variant(self):
        families = [f for path in TRANCHES for f in load_families(path)]
        ids = [f['family_id'] for f in families]
        self.assertEqual(len(ids), len(set(ids)))
        for example in build_examples(families):
            prompt, full = render(example)
            self.assertTrue(full.startswith(prompt) and full.endswith('<|im_end|>'))
            target = json.loads(example.target)
            self.assertEqual(set(target), {'alternatives', 'selected_option_id', 'justification',
                                           'uncertainty', 'revision_conditions'})
            self.assertEqual(target['selected_option_id'], example.selected)
            self.assertNotIn('{P', full); self.assertNotRegex(full, r'\[[a-z_]+\]')

    def test_no_family_crosses_splits_and_both_splits_exist(self):
        families = [f for path in TRANCHES for f in load_families(path)]
        self.assertEqual({f['split'] for f in families}, {'train', 'development'})
        for example in build_examples(families):
            self.assertEqual(example.split, next(f['split'] for f in families if f['family_id'] == example.family_id))

    def test_cosmetic_renames_and_permutes_but_keeps_the_same_original_decision(self):
        f = family()
        clean, cosmetic = variant(f, 'clean'), variant(f, 'cosmetic')
        self.assertNotIn(f['people']['P1'], cosmetic.user)
        self.assertIn(f['renamed']['P1'], cosmetic.user)
        self.assertNotEqual(clean.shown_to_original, cosmetic.shown_to_original)
        original = lambda e: dict(e.shown_to_original)[e.selected]
        self.assertEqual(original(clean), original(cosmetic))
        self.assertEqual(original(clean), f['selected'])

    def test_noise_keeps_the_decision_and_renumbers_cited_evidence(self):
        f = family()
        clean, noise = variant(f, 'clean'), variant(f, 'noise')
        self.assertGreater(noise.user.count('\nE'), clean.user.count('\nE'))
        self.assertEqual(dict(noise.shown_to_original)[noise.selected], f['selected'])
        cited = json.loads(noise.target)['justification']
        for ref in ('E1', 'E2', 'E3', 'E4', 'E5', 'E6'):
            if ref in cited:
                self.assertIn(ref+':', noise.user)

    def test_counterfactual_uses_its_own_key(self):
        f = family()
        cf = variant(f, 'counterfactual')
        self.assertEqual(dict(cf.shown_to_original)[cf.selected], f['counterfactual']['selected'])
        self.assertIn(f['counterfactual']['replace_evidence']['corte'].replace('{P1}', f['people']['P1']), cf.user)

    def test_contradictory_counterfactual_and_unknown_placeholders_refuse(self):
        f = family(); f['counterfactual']['changes'] = 'confidence'
        with self.assertRaises(AdmissionError): self.load([f])
        f = family(); f['justification'] += ' [no_existe]'
        with self.assertRaises(AdmissionError): build_examples(self.load([f]))
        f = family(); f['family_id'] = 'F001 con espacios'
        with self.assertRaises(AdmissionError): self.load([f])
        f = family(); f['provenance'] = {}
        with self.assertRaises(AdmissionError): self.load([f])

    def test_there_is_no_moral_admission_field(self):
        f = family(); f['moral_label'] = 'allowed'
        with self.assertRaises(AdmissionError): self.load([f])

    def test_labels_cover_only_target_tokens(self):
        example = variant(family(), 'clean')
        tokens = TokenExample(example, [10, 11, 12, 13, 14], prompt_tokens=3)
        self.assertEqual(tokens.labels(), [-100, -100, 13, 14, -100])

    def test_host_expansion_fills_objectives_and_actions_from_admitted_inputs(self):
        f = family(); example = variant(f, 'clean')
        options = {letter: 'acción '+letter for letter in LETTERS[:3]}
        analysis = expand_compact(json.loads(example.target), 'objetivo', 'semana', options)
        self.assertEqual(analysis['user_objectives'][0]['summary'], 'objetivo')
        self.assertEqual({a['action_summary'] for a in analysis['alternatives']}, set(options.values()))
        with self.assertRaises(AdmissionError):
            expand_compact(json.loads(example.target), 'objetivo', 'semana', {'A': 'x'})

    def load(self, families):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'t.json'
            path.write_text(json.dumps(families, ensure_ascii=False), encoding='utf-8')
            return load_families(path)
