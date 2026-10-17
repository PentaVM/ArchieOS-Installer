import json
from pathlib import Path
import tempfile
import unittest
from archie_wanda.chatdata import load_tranche, render, write_tranche
from archie_wanda.star import accept, answer_of, build_tranche, normalise
from archie_wanda.tomgen import generate_v2


class StarTests(unittest.TestCase):
    def example(self):
        return {'target': 'less full', 'example_id': 'x'}

    def test_answers_normalise_and_must_match_the_simulated_label(self):
        self.assertEqual(normalise(' Answer: Less full. '), 'less full')
        self.assertEqual(normalise('Respuesta: sí'), 'sí')
        good = {'text': 'Less full', 'thinking': 'Hazel saw it...', 'thinking_tokens': 300, 'forced_end': False}
        self.assertTrue(accept(self.example(), good, 640))
        self.assertFalse(accept(self.example(), dict(good, text='more full'), 640))
        self.assertFalse(accept(self.example(), dict(good, forced_end=True), 640))
        self.assertFalse(accept(self.example(), dict(good, thinking_tokens=900), 640))
        self.assertFalse(accept(self.example(), dict(good, thinking=' '), 640))
        self.assertEqual(answer_of({'target': 'trace\n</think>\n\nyes'}), 'yes')

    def test_built_tranche_keeps_only_accepted_traces_in_think_mode(self):
        problems = generate_v2(40, seed=5, direct_share=1.0)
        with tempfile.TemporaryDirectory() as d:
            source, out = Path(d)/'src.json', Path(d)/'star.json'
            examples = write_tranche(problems, source, {'kind': 'owned-procedural'})
            results = {examples[0]['example_id']: {'text': examples[0]['target'], 'thinking': 'my own reasoning',
                                                   'thinking_tokens': 50, 'forced_end': False},
                       examples[1]['example_id']: {'text': 'wrong', 'thinking': 'x', 'thinking_tokens': 5,
                                                   'forced_end': False}}
            summary = build_tranche(source, results, out, 640)
            data = load_tranche(out)
        self.assertEqual(summary, {'attempted': 2, 'accepted': 1})
        kept = data['examples'][0]
        self.assertEqual(kept['mode'], 'think')
        prompt, full = render(kept)
        self.assertTrue(prompt.endswith('<think>\n'))
        self.assertTrue(full.endswith('my own reasoning\n</think>\n\n' + examples[0]['target'] + '<|im_end|>'))
