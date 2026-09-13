# Visual Cortex

Visual Cortex is an event-driven advisory interface for ambiguous, task-requested ROIs. A
request contains references and bounded keyframes; outputs are proposals and cannot add
scene nodes the task did not request. Privacy and locality are checked before provider use,
so restricted state cannot reach cloud inference.

No 6B vision model is bundled. Qwen3-VL LoRA/QLoRA integration and a real labelled
evaluation are **BLOCKED** on a selected checkpoint, licensed corpus and GPU budget.
