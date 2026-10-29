# Project ownership

Work exclusively in `PentaVM/ArchieOS-Nvidia` for this hackathon. The older
`ArchieOS` repository is historical; do not continue features there. Verify the
remote before editing, integrating branches or pushing.

Preserve Core, Cortex/Cerebra, IRIS, Orbe, Policy, leases and the existing authority
boundaries. Voice is the main interaction. Audit existing implementations before
adding replacements. Keep personal and work domains separate.

The professional Quickshell/Vault GUI, the neural sphere and WSL generation have two
authors: Xavi (`ByXavi270` / `XaviCottier`) for the design and PentaVM for the native
implementation in Rust/C++. Credit both. Do not overwrite their uncommitted files. The
GUI handoff is context for integration; implementation work on it is PentaVM's to direct.

Santiago Rearte (Sheriff) brought in the Nebius Token Factory / Nemotron integration and
tests the node on his own WSL. Credit him alongside Xavi and PentaVM.

Integrate completed branches using their changes relative to main, preserve other
agents' worktrees, test the affected paths and record context/results in `_dev`.
Do not declare hardware, model accuracy, latency or deployment verified by mocks.
