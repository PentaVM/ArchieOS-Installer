import unittest
from archie_wanda.native_chat import NativeChat, render_chat


class FakeChat(NativeChat):
    """Replays scripted completions: each call returns (text, tokens, stopped_on_word)."""

    def __init__(self, script):
        self.script, self.prompts = list(script), []

    def _complete(self, prompt, n_predict, stop, seed):
        self.prompts.append((prompt, n_predict, tuple(stop)))
        text, tokens, stopped = self.script.pop(0)
        return text, min(tokens, n_predict), stopped


MESSAGES = [{'role': 'system', 'content': 'Sys'}, {'role': 'user', 'content': 'Q?'}]


class RenderTests(unittest.TestCase):
    def test_matches_the_base_chat_template(self):
        self.assertEqual(render_chat(MESSAGES),
                         '<|im_start|>system\nSys<|im_end|>\n<|im_start|>user\nQ?<|im_end|>\n<|im_start|>assistant\n')
        with self.assertRaises(ValueError):
            render_chat([{'role': 'assistant', 'content': 'x'}])


class BudgetForcingTests(unittest.TestCase):
    def test_natural_end_within_budget_keeps_the_thinking(self):
        chat = FakeChat([('reasoning', 100, True), ('Answer A', 3, True)])
        out = chat.think(MESSAGES, budget=512)
        self.assertEqual(out['text'], 'Answer A')
        self.assertFalse(out['forced_end'])
        self.assertTrue(chat.prompts[1][0].endswith('reasoning\n</think>\n\n'))

    def test_budget_exhaustion_forces_the_end_of_thinking(self):
        chat = FakeChat([('long reasoning', 512, False), ('No', 1, True)])
        out = chat.think(MESSAGES, budget=512)
        self.assertTrue(out['forced_end'])
        self.assertEqual(out['thinking_tokens'], 512)
        self.assertEqual(chat.prompts[1][0].count('</think>'), 1)

    def test_early_stop_is_extended_with_wait_up_to_the_limit(self):
        chat = FakeChat([('short', 10, True), (' more', 20, True), (' done', 30, True), ('Yes', 1, True)])
        out = chat.think(MESSAGES, budget=512, min_think=200, max_waits=2)
        self.assertEqual(out['waits'], 2)
        self.assertEqual(out['thinking'], 'shortWait moreWait done')
        self.assertEqual(chat.prompts[1][1], 512 - 10)       # the remaining budget is passed on
