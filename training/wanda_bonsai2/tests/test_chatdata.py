import json
from pathlib import Path
import tempfile
import unittest
from archie_wanda.chatdata import load_tranche, render, world_split, write_tranche
from archie_wanda.contracts import AdmissionError
from archie_wanda.tomgen import generate


class ChatDataTests(unittest.TestCase):
    def test_round_trip_split_by_world_and_render(self):
        problems = generate(200, seed=11)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d)/'t.json'
            write_tranche(problems, path, {'kind': 'owned-procedural', 'seed': 11})
            data = load_tranche(path)
        splits = {}
        for e in data['examples']:
            self.assertEqual(splits.setdefault(e['family_id'], e['split']), e['split'])
        self.assertIn('development', set(splits.values()))
        prompt, full = render(data['examples'][0])
        self.assertTrue(prompt.endswith('<|im_start|>assistant\n<think>\n'))
        self.assertTrue(full.endswith('<|im_end|>') and '</think>' in full[len(prompt):])

    def test_direct_mode_renders_an_empty_think_block_and_the_bare_answer(self):
        from archie_wanda.tomgen import generate_v2
        problems = [p for p in generate_v2(60, seed=3) if p.mode == 'direct'][:3]
        with tempfile.TemporaryDirectory() as d:
            path = Path(d)/'t.json'
            examples = write_tranche(problems, path, {'kind': 'owned-procedural'})
        prompt, full = render(examples[0])
        self.assertTrue(prompt.endswith('<think>\n\n</think>\n\n'))
        self.assertEqual(full[len(prompt):], problems[0].answer + '<|im_end|>')

    def test_split_is_stable_per_world(self):
        self.assertEqual(world_split('w1-7'), world_split('w1-7'))

    def test_world_crossing_splits_and_missing_provenance_refuse(self):
        problems = generate(20, seed=2)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d)/'t.json'
            write_tranche(problems, path, {'kind': 'owned-procedural'})
            data = json.loads(path.read_text())
            data['examples'][1]['family_id'] = data['examples'][0]['family_id']
            data['examples'][1]['split'] = 'development' if data['examples'][0]['split'] == 'train' else 'train'
            path.write_text(json.dumps(data))
            with self.assertRaises(AdmissionError): load_tranche(path)
            data['provenance'] = {}
            path.write_text(json.dumps(data))
            with self.assertRaises(AdmissionError): load_tranche(path)
