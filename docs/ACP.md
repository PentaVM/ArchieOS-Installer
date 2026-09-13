# Archie Context Protocol

ACP is a compact typed projection of requested scene state. A checkpoint names the source
revision and selected nodes; deltas contain only changed/deleted selected nodes; an action
horizon bounds anticipated work. Secret state is rejected before encoding.

`benchmark_sizes` reports bytes and structural counts. It claims token counts only when
an explicitly named tokenizer adapter is supplied. Target-model tokenizer benchmarking is
**BLOCKED** until the deployment model is selected and installed.
