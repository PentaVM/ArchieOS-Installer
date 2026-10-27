# Per-installation package keyring plan

Goal: remove the two audited releases and ensure each new WSL installation creates
its own pacman identity. Execution authorized by the user's 2026-10-27 request.

Design: preserve the existing offline first-boot flow and packaging rejection.
Strip the builder's entire pacman keyring only from a checked staging root, reject
populated keyrings at every packaging entry, and initialize/populate from the
installed public Arch keyrings at first boot. The PowerShell import paths invoke
the same initializer explicitly. Repeated initialization preserves the installed
machine's identity. Existing Core/Policy/voice and GUI behavior are outside scope.

Use the existing isolated audit clone on codex/per-install-pacman-keyring.
Implement inline. No new GUI, full WSL rebuild, or changes to another checkout.

- [x] Delete the two releases, all their assets, and their exact tags; verify 404
  and preserve the unrelated draft. Receipt retained outside the repo.
- [x] Add failing tests for staged-keyring rejection/cleanup, path containment,
  packaging archive contents, and initializer failure propagation/idempotence.
- [x] Add scripts/wsl/image-keyring.py with prepare/check operations and safe
  staging-root containment. Reuse it from both builders and both packagers.
- [x] Add the shared first-boot initializer to the operator overlay, install it
  in the workstation variant, wire OOBE and both PowerShell import paths.
- [x] Run affected packaging and initializer checks; inspect the final diff and
  scan changed files for secrets. Record actual limitations in _dev.
- [x] Delivered security commit 5759b62 to GitHub main. Removal verification passed;
  CI run 40456703332 could not start due to account billing/spending restrictions.

Review focus: no writes to a live host keyring; symlink escape must fail before
cleanup; output archives must contain no builder identity/trust; initializer
failure must stop install; repeated first boot must not rotate a local key.

History: actual leaked GnuPG keys were release assets, not Git blobs. Removing
those assets/tags does not require rewriting unrelated source commits. GitHub
caches, previously issued download URLs and existing downloaded copies cannot be
guaranteed erased through release deletion alone.
