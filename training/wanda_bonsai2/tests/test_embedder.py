import unittest
from archie_wanda import bench

HAVE_MODEL = (bench.BENCH/'models/all-roberta-large-v1/model.safetensors').exists()


@unittest.skipUnless(HAVE_MODEL, 'official FANToM embedder lives only on the WSL run host')
class EmbedderTests(unittest.TestCase):
    def test_cosine_ordering_is_sane(self):
        embed = bench.official_embedder()
        same, close, far = embed(['Kailey believes the trip was in May.', 'Kailey believes the trip was in May.',
                                  'Kailey thinks the vacation happened in May.'])[0], None, None
        a, b, c, d = embed(['Kailey believes the trip was in May.', 'Kailey believes the trip was in May.',
                            'Kailey thinks the vacation happened in May.', 'The stock market fell on Tuesday.'])
        self.assertAlmostEqual(float(a @ b), 1.0, places=5)
        self.assertGreater(float(a @ c), float(a @ d))
        self.assertEqual(a.shape, (1024,))
