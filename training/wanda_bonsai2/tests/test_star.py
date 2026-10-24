import json
from pathlib import Path
import tempfile
import unittest
from archie_wanda.chatdata import load_tranche, render, write_tranche
from archie_wanda.star import accept, answer_of, build_tranche, hinted, normalise
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

    def test_answer_may_open_with_the_label_but_not_with_a_longer_word(self):
        good = {'thinking': 'x', 'thinking_tokens': 10, 'forced_end': False}
        no = {'target': 'no', 'example_id': 'n'}
        self.assertTrue(accept(no, dict(good, text='No. Hugo saw the move.'), 640))
        self.assertTrue(accept(no, dict(good, text='no, because he left'), 640))
        self.assertFalse(accept(no, dict(good, text='Yes. Hugo left before the move.'), 640))
        self.assertFalse(accept(no, dict(good, text='Nobody knows'), 640))
        self.assertFalse(accept(no, dict(good, text='No. Correction: yes.'), 640))
        less = {'target': 'less full', 'example_id': 'l'}
        self.assertFalse(accept(less, dict(good, text='less'), 640))
        self.assertTrue(accept(less, dict(good, text='Less full: she drank some.'), 640))
        self.assertTrue(accept(less, dict(good, text='Reasoning: she drank some.\n**Answer:** Less full.'), 640))
        self.assertTrue(accept(no, dict(good, text='Hugo saw the move.\n\nNo'), 640))

    def test_rationalization_hints_the_label_but_trains_on_the_original_prompt(self):
        problems = generate_v2(40, seed=5, direct_share=1.0)
        with tempfile.TemporaryDirectory() as d:
            source, out = Path(d)/'src.json', Path(d)/'star.json'
            examples = write_tranche(problems, source, {'kind': 'owned-procedural'})
            a, b, c = examples[:3]
            prompt = hinted(a)
            self.assertIn(answer_of(a), prompt[-1]['content'][len(a['messages'][-1]['content']):])
            self.assertEqual(prompt[:-1], a['messages'][:-1])
            ok = {'thinking_tokens': 50, 'forced_end': False}
            results = {e['example_id']: dict(ok, text='wrong', thinking='x') for e in (a, b, c)}
            results[c['example_id']] = dict(ok, text=c['target'], thinking='own reasoning')
            rationales = {a['example_id']: dict(ok, text=a['target'], thinking='step one, despistada step two'),
                          b['example_id']: dict(ok, text=b['target'], thinking='The hint says so.'),
                          c['example_id']: dict(ok, text=c['target'], thinking='rationalized')}
            summary = build_tranche(source, results, out, 640, rationales)
            data = load_tranche(out)
        self.assertEqual((summary['self'], summary['rationalized']), (1, 1))
        kept = {e['example_id']: e for e in data['examples']}
        self.assertEqual(kept[a['example_id']]['trace'], 'rationalized')
        self.assertEqual(kept[a['example_id']]['messages'], a['messages'])
        self.assertNotIn(b['example_id'], kept)
        self.assertTrue(kept[c['example_id']]['target'].startswith('own reasoning'))

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
        self.assertEqual(summary, {'attempted': 2, 'accepted': 1, 'self': 1, 'rationalized': 0})
        kept = data['examples'][0]
        self.assertEqual(kept['mode'], 'think')
        prompt, full = render(kept)
        self.assertTrue(prompt.endswith('<think>\n'))
        self.assertTrue(full.endswith('my own reasoning\n</think>\n\n' + examples[0]['target'] + '<|im_end|>'))
