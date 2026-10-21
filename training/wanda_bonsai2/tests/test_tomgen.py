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
                self.assertTrue(p.target().endswith('\n</think>\n\n' + p.answer))
                p.mode = 'direct'
                self.assertEqual(p.target(), p.answer)

    def test_spanish_contractions_and_capitals(self):
        for w in self.worlds('es', 200):
            text = w.story() + ' '.join(q.question + ' '.join(q.trace) for q in w.problems())
            self.assertIsNone(re.search(r'\bde el\b|\ba el\b', text))
            for sentence in w.lines:
                self.assertTrue(sentence[0].isupper(), sentence)


class MinimalPairTests(unittest.TestCase):
    def test_twins_differ_only_in_the_forced_variable_and_flip_the_watcher(self):
        from archie_wanda.tomgen import minimal_pairs
        flips = 0
        for seed in range(100):
            sensitive, invariant = minimal_pairs(seed, 'en', f'p{seed}')
            for a, b in sensitive + invariant:
                self.assertEqual(a.meta['pair'], b.meta['pair'])
                self.assertEqual(a.question, b.question)
                story_a = set(re.split(r'(?<=\.) ', a.story)); story_b = set(re.split(r'(?<=\.) ', b.story))
                self.assertLessEqual(len(story_a ^ story_b), 2)      # leave (+ come back) only
            flips += len(sensitive)
            self.assertTrue(invariant)                               # e.g. the mover's own belief
        self.assertGreater(flips, 300)

    def test_v2_is_reproducible_across_processes(self):
        import subprocess, sys, os
        code = ('import sys; sys.path.insert(0, "src"); from archie_wanda.tomgen import generate_v2; '
                'print("|".join(p.question + p.label for p in generate_v2(120, seed=9)))')
        runs = {subprocess.run([sys.executable, '-c', code], capture_output=True, text=True,
                               env=dict(os.environ, PYTHONHASHSEED=str(seed))).stdout for seed in (1, 2)}
        self.assertEqual(len(runs), 1)

    def test_v2_mix_has_pairs_both_modes_and_unique_ids(self):
        from archie_wanda.tomgen import generate_v2
        problems = generate_v2(400, seed=7)
        self.assertEqual(len(problems), 400)
        self.assertEqual({p.mode for p in problems}, {'think', 'direct'})
        self.assertGreaterEqual(sum('pair' in p.meta for p in problems), 180)


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


class BehavioralAndPackTests(unittest.TestCase):
    def test_behavioral_world_faux_pas_and_emotional_consequences(self):
        from archie_wanda.tomgen import BehavioralWorld, behavioral_minimal_pairs
        for seed in range(40):
            sensitive, _ = behavioral_minimal_pairs(seed, 'en', f'bp{seed}')
            self.assertEqual(len(sensitive), 3)
            for a, b in sensitive:
                self.assertNotEqual(a.label, b.label)
        w = BehavioralWorld(random.Random(1), 'es', 'bw1',
                            force={'proposer_present_at_disclosure': False,
                                   'reply_style': 'tentative',
                                   'proposer_response': 'push'})
        by_kind = {p.kind: p for p in w.problems()}
        self.assertEqual(by_kind['faux_pas_aware'].label, 'no')
        self.assertEqual(by_kind['commitment'].label, 'tentative')
        self.assertEqual(by_kind['emotional_effect'].label, 'pressured')

    def test_pack_state_world_pairs_and_generate_v3(self):
        from archie_wanda.tomgen import generate_v3, pack_minimal_pairs
        pairs = pack_minimal_pairs(11, 'pp11')
        self.assertEqual(len(pairs), 2)
        for a, b in pairs:
            self.assertNotEqual(a.label, b.label)
        problems = generate_v3(240, seed=19)
        self.assertEqual(len(problems), 240)
        kinds = {p.kind for p in problems}
        self.assertTrue({'faux_pas_aware', 'commitment', 'emotional_effect', 'pack_knowledge', 'pack_belief'}.issubset(kinds))

