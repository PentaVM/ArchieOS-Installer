import unittest
from archie_wanda.worker import Worker


class EpochOrderTests(unittest.TestCase):
    def train(self):
        rows = [{'pair': f'p{i // 2}'} for i in range(10)] + [{'pair': None} for _ in range(6)]
        return rows

    def test_pairs_stay_adjacent_and_every_example_appears_once(self):
        train = self.train()
        order = Worker.epoch_order(train, 17, 0, keep_pairs=True)
        self.assertEqual(sorted(order), list(range(len(train))))
        for pos, i in enumerate(order):
            pair = train[i]['pair']
            if pair and train[order[pos - 1]]['pair'] != pair:
                self.assertEqual(train[order[pos + 1]]['pair'], pair)

    def test_order_is_deterministic_per_epoch_and_plain_mode_unchanged(self):
        train = self.train()
        self.assertEqual(Worker.epoch_order(train, 17, 1, True), Worker.epoch_order(train, 17, 1, True))
        self.assertNotEqual(Worker.epoch_order(train, 17, 1, True), Worker.epoch_order(train, 17, 2, True))
        import random
        expected = list(range(len(train))); random.Random('17:0').shuffle(expected)
        self.assertEqual(Worker.epoch_order(train, 17, 0, False), expected)
