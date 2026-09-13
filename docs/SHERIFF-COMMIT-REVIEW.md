# Sheriff commit review

## Commit

- SHA: `9a2ab7ba0d530936a985e1f7fcf6612603bc0421`
- Author: Santiago Rearte `<santyrearte@gmail.com>`
- Subject: `Integration of the Nebius API and selection of the Nemotron-3-Super-120B-A12B model.`
- Reviewed against parent: `d51276576fc00a6ed71872b42318ea4686e14a9e`

## Apparent objective

Add a Nebius Token Factory client and a runnable Nemotron engineering agent with local
filesystem and command tools.

## What is sound

- Credentials are read from `NEBIUS_TOKEN_FACTORY_KEY` or `NEBIUS_API_KEY`; no key was
  committed and `.env` is ignored.
- The prototype uses the OpenAI-compatible Token Factory endpoint, a finite request
  timeout, and basic HTTP error handling.
- File reads and subprocess duration have elementary bounds.
- The commit did not alter the validated Cerebra, Vault, floor, policy, or executor
  contracts.

## What must change

- `core/crates/archie-model-runtime/src/nebius.rs` is not declared by `lib.rs`. It is
  therefore dead code and is absent from the crate's nine passing tests.
- If declared, that file would not compile: it imports `RuntimeError` from
  `archie_types` and uses error variants that the active runtime does not define. It
  also duplicates the stricter Nebius implementation already in `lib.rs`.
- `tools/nemotron_agent.py` lets a remote model invoke reads, writes, directory listing,
  and `subprocess.run(shell=True)` directly. It bypasses Cerebra, Policy, capability
  leases, and the executor.
- Its path joins do not canonicalize and prove containment. Absolute paths and `..` can
  leave the intended project root.
- Its privacy decision is a keyword search. Absence of words such as `secret` or
  `token` cannot authorize disclosure.
- It prints tool arguments, provider errors, model reasoning, and model content. Those
  surfaces can disclose private material.
- It hardcodes a model ID and resends an unbounded conversation history instead of
  using router discovery and task-bound context capsules.
- `project_structure.md` is a point-in-time listing rather than an executable contract.

## Risks

- Workspace escape and arbitrary shell execution.
- Private file disclosure to a cloud model.
- The model becoming execution authority instead of an adviser.
- False integration coverage because the added Rust file is not compiled.
- Runtime breakage when a hardcoded model name changes upstream.

## Compatibility

The intent to use Nebius is compatible with the current system. The implementation is
not compatible with Cerebra's authority invariant or the WSL workstation boundary in
its current form. The active provider-neutral `InferenceClient` is the base to keep:
it already checks provider, locality, privacy grade, redirects, response size, and
runtime credentials.

## Verdict

**YES WITH CHANGES.** Preserve the commit and its provider intent. Retire the direct
executor prototype and compose Nebius through the existing router, context gateway,
policy, capability lease, and executor.

