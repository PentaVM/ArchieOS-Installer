from collections import Counter
import random
import re
import unittest
from archie_wanda.tomgen import LABELS, ConversationWorld, ObjectWorld, T, generate


class ObjectWorldTests(unittest.TestCase):
    def worlds(self, lang='en', n=300):
        return [ObjectWorld(random.Random(i), lang, f'w{i}') for i in range(n)]

    def test_first_order_belief_follows_observation_or_telling(self):
        for w in self.worlds():
            self.assertTrue(w.knows_move(w.mover))
            self.assertEqual(w.knows_move(w.watcher), w.saw[w.watcher] or w.watcher in w.told)
            for p in w.problems():
                if p.kind == 'location_coarse_fo' and p.question.startswith(f'Does {w.watcher} '):
                    self.assertEqual(p.label, 'no' if w.knows_move(w.watcher) else 'yes')

    def test_second_order_cases(self):
        seen = set()
        for w in self.worlds():
            # the mover's view of the watcher is the watcher's true state
            self.assertEqual(w.thinks_other_knows(w.mover, w.watcher), w.knows_move(w.watcher))
            # the watcher thinks the mover knows iff the watcher learned of the move at all
            self.assertEqual(w.thinks_other_knows(w.watcher, w.mover), w.knows_move(w.watcher))
            seen.add((w.saw[w.watcher], w.watcher in w.told))
        self.assertEqual(seen, {(True, False), (False, True), (False, False)})

    def test_fullness_and_accessibility_are_unchanged_for_someone_unaware(self):
        for w in self.worlds():
            if w.knows_move(w.watcher):
                continue
            for p in w.problems():
                if p.kind in ('fullness', 'accessibility') and p.question.startswith(f"From {w.watcher}'s"):
                    self.assertEqual(p.label, 'same')

    def test_targets_have_a_closed_trace_and_the_answer(self):
        for w in self.worlds('es', 50):
            for p in w.problems():
                self.assertIn('\n</think>\n\n', p.target())
                self.assertTrue(p.target().endswith('Respuesta: ' + p.answer))

    def test_spanish_contractions_and_capitals(self):
        for w in self.worlds('es', 200):
            text = w.story() + ' '.join(q.question + ' '.join(q.trace) for q in w.problems())
            self.assertIsNone(re.search(r'\bde el\b|\ba el\b', text))
            for sentence in w.lines:
                self.assertTrue(sentence[0].isupper(), sentence)


class ConversationTests(unittest.TestCase):
    def test_knowledge_matches_presence_at_the_update(self):
        for i in range(300):
            w = ConversationWorld(random.Random(i), 'en', f'c{i}')
            for p in w.problems():
                if p.kind == 'knowledge':
                    name = p.question.split()[1]
                    self.assertEqual(p.label == 'yes', name in w.heard)
            # nobody is asked about who never appears in the text
            self.assertTrue(all(any(n in line for line in w.lines) for n in w.mentioned))


class GenerateTests(unittest.TestCase):
    def test_labels_are_balanced_so_constant_answers_fail(self):
        problems = generate(2000, seed=1)
        self.assertEqual(len(problems), 2000)
        for (kind, lang), group in self.group(problems).items():
            counts = Counter(p.label for p in group)
            if len(group) >= 30:
                self.assertLessEqual(max(counts.values()) - min(counts.get(l, 0) for l in LABELS[kind]), 2, (kind, lang, counts))
        self.assertEqual({p.lang for p in problems}, {'en', 'es'})
        self.assertGreater(len({p.kind for p in problems}), 8)

    def test_deterministic_and_worlds_never_repeat_within_a_run(self):
        a, b = generate(300, seed=5), generate(300, seed=5)
        self.assertEqual([p.question for p in a], [p.question for p in b])
        self.assertLessEqual(max(Counter(p.world_id for p in a).values()), 4)

    @staticmethod
    def group(problems):
        out = {}
        for p in problems:
            if p.kind in LABELS:
                out.setdefault((p.kind, p.lang), []).append(p)
        return out
