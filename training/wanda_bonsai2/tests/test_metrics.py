from dataclasses import asdict
from pathlib import Path
import unittest
from archie_wanda.data import build_examples, load_families, probe_text, render
from archie_wanda.metrics import aggregate, shown_options, trivial_baselines

DATA = Path(__file__).resolve().parents[1]/'data'/'social'


def development():
    families = [f for path in sorted(DATA.glob('tranche-00[12].json')) for f in load_families(path)]
    return [asdict(e) for e in build_examples(families) if e.split == 'development']


class MetricsTests(unittest.TestCase):
    def record(self, variant, pred, selected='A', mapping=(('A', 'A'), ('B', 'B')), changes='action'):
        return {'family_id': 'f', 'variant': variant, 'pred': pred, 'selected': selected,
                'shown_to_original': mapping, 'counterfactual_changes': changes}

    def test_family_counts_only_when_every_variant_is_right(self):
        good = [self.record('clean', 'A'), self.record('noise', 'A'),
                self.record('cosmetic', 'A'), self.record('counterfactual', 'B', selected='B')]
        result = aggregate(good)
        self.assertEqual(result['family_all_correct'], 1.0)
        self.assertEqual(result['counterfactual_flip_rate'], 1.0)
        bad = good[:3] + [self.record('counterfactual', 'A', selected='B')]
        result = aggregate(bad)
        self.assertEqual(result['family_all_correct'], 0.0)
        self.assertEqual(result['counterfactual_flip_rate'], 0.0)
        self.assertEqual(result['accuracy_counterfactual'], 0.0)

    def test_cosmetic_invariance_compares_original_options_not_letters(self):
        records = [self.record('clean', 'A'), self.record('noise', 'A'),
                   self.record('cosmetic', 'B', selected='B', mapping=(('A', 'B'), ('B', 'A'))),
                   self.record('counterfactual', 'B', selected='B')]
        self.assertEqual(aggregate(records)['cosmetic_invariance'], 1.0)

    def test_shown_options_reads_the_prompt(self):
        example = development()[0]
        options = shown_options(example['user'])
        self.assertEqual(sorted(options), list(example['option_letters']))
        self.assertTrue(all(text for text in options.values()))

    def test_trivial_baselines_expose_the_length_shortcut(self):
        baselines = trivial_baselines(development())
        self.assertEqual(set(baselines), {'longest_option', 'shortest_option', 'always_A', 'random'})
        # Independent audit (Codex, 2026-10-16): the longest option wins 25/32 dev variants.
        self.assertAlmostEqual(baselines['longest_option']['choice_accuracy'], 25/32)

    def test_probe_has_no_gold_text(self):
        example = build_examples(load_families(sorted(DATA.glob('tranche-001.json'))[0]))[0]
        probe = probe_text(example)
        prompt, _ = render(example)
        self.assertTrue(probe.startswith(prompt))
        self.assertEqual(probe[len(prompt):], '{"selected_option_id":"')
