from collections import Counter
import unittest
from archie_wanda import longchat


class LongChatTests(unittest.TestCase):
    def test_knowledge_follows_presence_at_the_last_update(self):
        world = longchat.build(5, target_chars=20000)
        self.assertTrue(world['updates'])
        for topic, _, _ in world['facts']:
            for p in world['present']:
                # whoever is still present at the end saw any update made while present
                if world['knowledge'][topic][p] is not None:
                    self.assertIn(world['knowledge'][topic][p], longchat._values(world, topic))

    def test_pack_is_much_shorter_and_keeps_every_update(self):
        world = longchat.build(9, target_chars=84000)
        raw, packed = '\n'.join(world['lines']), longchat.pack(world)
        self.assertLess(len(packed), len(raw) / 3)
        self.assertEqual(sum('UPDATE' in l for l in world['ledger']), len(world['updates']))

    def test_pack_v2_states_each_persons_view_consistently_with_the_simulation(self):
        world = longchat.build(9, target_chars=30000)
        text = longchat.pack_state(world)
        for topic, verb, _ in world['facts']:
            for p in world['people']:
                seen = world['knowledge'][topic][p]
                if seen is not None:
                    self.assertIn(f'{p}: last saw that {topic} {verb.format(v=seen)}', text)
        three = [longchat.items(worlds=3, target_chars=30000, form=f) for f in ('raw', 'pack', 'pack2')]
        golds = [[(i['id'].split(':', 1)[1], i['gold']) for i in items] for items in three]
        self.assertEqual(golds[0], golds[1]); self.assertEqual(golds[0], golds[2])

    def test_both_forms_ask_identical_questions_and_constant_answers_fail(self):
        raw = longchat.items(worlds=6, target_chars=30000, form='raw')
        packed = longchat.items(worlds=6, target_chars=30000, form='pack')
        strip = lambda i: (i['id'].split(':', 1)[1], i['gold'])
        self.assertEqual([strip(i) for i in raw], [strip(i) for i in packed])
        gold = Counter(i['gold'] for i in raw if i['kind'] == 'knowledge')
        self.assertEqual(gold['yes'], gold['no'])
        always_yes = longchat.score(raw, {i['id']: 'yes' for i in raw})['knowledge']['accuracy']
        self.assertAlmostEqual(always_yes, 0.5)
