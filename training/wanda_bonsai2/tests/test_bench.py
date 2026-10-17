import unittest
from archie_wanda import bench

HAVE_DATA = (bench.BENCH/'data/fantom/fantom_v1.json').exists() and (bench.BENCH/'src/opentom').exists()


class MetricTests(unittest.TestCase):
    def test_macro_and_weighted_f1_match_sklearn_reference_values(self):
        gold, pred = [0, 0, 1, 1, 2, 2], [0, 1, 1, 1, 2, 0]
        # sklearn.metrics.f1_score(gold, pred, average='macro') == 0.6555555555555556
        self.assertAlmostEqual(bench.f1_macro(gold, pred), 0.6555555555555556)
        # average='weighted' (equal supports) is the same here
        self.assertAlmostEqual(bench.f1_weighted(gold, pred), 0.6555555555555556)
        # an unparsed prediction label (-1) joins the label set with zero support
        self.assertAlmostEqual(bench.f1_weighted([1, 1, 0], [1, -1, 0]), (2/3*2 + 1.0)/3)

    def test_fantom_parsers_follow_the_official_rules(self):
        self.assertTrue(bench._mc({'correct_answer': 1}, '(b) Kim thinks so'))
        self.assertTrue(bench._mc({'correct_answer': 0}, 'a. yes'))
        self.assertFalse(bench._mc({'correct_answer': 0}, '(b)'))
        self.assertEqual(bench._binary('Yes, she does.'), 1)
        self.assertEqual(bench._binary('No.'), 0)
        self.assertEqual(bench._binary('Maybe'), -1)
        self.assertEqual(bench.parse_fantom_response('blah Answer: Linda'), 'Linda')

    def test_fantom_set_scores_require_every_question_in_the_set(self):
        items, preds = [], {}
        def add(i, set_id, qtype, correct, wrong=None, pred='', access='inaccessible'):
            items.append({'id': str(i), 'set_id': set_id, 'qa': {'set_id': set_id, 'question_type': qtype,
                          'correct_answer': correct, 'wrong_answer': wrong, 'missed_info_accessibility': access}})
            preds[str(i)] = pred
        add(0, 's1', 'tom:answerability:list', ['Ann'], ['Bob'], 'Ann')
        add(1, 's1', 'tom:answerability:binary', 'yes', pred='yes')
        add(2, 's2', 'tom:answerability:list', ['Ann'], ['Bob'], 'Ann and Bob')
        add(3, 's2', 'tom:answerability:binary', 'no', pred='no')
        report = bench.fantom_score(items, preds)
        self.assertEqual(report['inaccessible:answerability:set:ALL'], 0.5)
        self.assertEqual(report['inaccessible:answerability:list'], 0.5)

    def test_free_form_belief_without_embedder_is_not_estimated(self):
        items = [{'id': '0', 'set_id': 's', 'qa': {'set_id': 's', 'question_type': 'tom:belief:inaccessible',
                  'correct_answer': 'x', 'wrong_answer': 'y', 'missed_info_accessibility': 'inaccessible'}}]
        report = bench.fantom_score(items, {'0': 'x'})
        self.assertIsNone(report['inaccessible:set:ALL*'])
        self.assertIsNone(report['inaccessible:belief:distance'])


@unittest.skipUnless(HAVE_DATA, 'benchmark data lives only on the WSL run host')
class DataTests(unittest.TestCase):
    def test_fantom_items_are_complete_and_deterministic(self):
        first, second = bench.fantom_items(), bench.fantom_items()
        self.assertEqual(len(first), 12832)
        self.assertEqual([i['messages'] for i in first[:50]], [i['messages'] for i in second[:50]])
        mc = [i for i in first if i['qa']['question_type'].endswith(':multiple-choice')]
        self.assertTrue(all(i['qa']['correct_answer'] in (0, 1) for i in mc))
        self.assertTrue(0.4 < sum(i['qa']['correct_answer'] for i in mc)/len(mc) < 0.6)

    def test_opentom_items_follow_the_official_sampling(self):
        items = bench.opentom_items()
        self.assertEqual(len({i['key'] for i in items}), 250)
        self.assertEqual({i['batch'] for i in items}, {1, 2, 3, 4, 5})
        self.assertEqual(bench.opentom_items()[0]['messages'], items[0]['messages'])
        self.assertEqual(items[0]['messages'][0]['role'], 'system')
