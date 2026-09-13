# Cerebra clarification

`archie-cerebra::clarification` is a deterministic gate after context inspection and
before a consequential proposal is committed. It uses permille integers for ambiguity,
missing information, impact, reversibility, risk, confidence, wrong-assumption cost,
interruption cost, expected information gain and latency cost.

The result is one of `CONTINUE`, `ASK_NOW`, `ASK_WHEN_FLOOR_AVAILABLE`,
`ASK_BEFORE_COMMIT`, `ASK_BEFORE_EXTERNAL_EFFECT`, or `DEFER_AND_WORK`. A question that
would otherwise be asked now becomes `ASK_WHEN_FLOOR_AVAILABLE` when the existing floor
policy has not made speech available. The clarification gate does not estimate the floor
and does not change `decide_floor`.

Persisted session state contains only a reason code, consulted refs, answer refs,
confidence delta and task outcome. It has no question or answer text field. Repeating an
already asked reason returns `DEFER_AND_WORK`, preventing a redundant question loop.

