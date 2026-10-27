# Keyring remediation — 2026-10-27

User authorized removing both audited releases and generating a separate pacman
keyring for each installation through the existing installer.

## GitHub cleanup completed

- Deleted releases `409929971` and `410201568`, including their nine assets.
- GET by every removed release/asset ID returned HTTP 404 after deletion.
- Removed the published release's tag and verified both affected tag names are
  absent from GitHub. Removed the old tag from this isolated local clone too.
- The unrelated newer draft `416918011` remains a draft.
- Checked all 4,965 local Git blobs, including fetched branch/tag/PR history,
  against distinctive private-exponent fragments from the two exposed GnuPG
  keys: zero matches. No source-history rewrite was necessary. The actual private
  key material was in the deleted release assets.

The deletion receipt and original audit corpus are retained privately outside the
repository in `D:\CodexAudits\ArchieOS-Nvidia-20261026`. No key values are in this
record. API deletion does not guarantee removal from GitHub caches, previously
issued signed download URLs or copies already downloaded by third parties.

## Source change

Both builders clear the entire local pacman identity/trust from a separate,
validated staging root immediately before packaging. A shared check rejects a
populated keyring in the standalone offline/workstation packagers. It refuses a
live filesystem root and symlink/junction traversal before recursive cleanup.
Public Arch key definitions in `/usr/share/pacman/keyrings` are preserved.

`init-pacman-keyring` runs as root at first boot/OOBE and explicitly after the
PowerShell import. It initializes the machine-local keyring and populates the
installed public Arch keys. It does not delete or rotate the installed identity
on repeated invocations. Installation stops if initialization fails.

The operator image installs the helper as executable; the workstation builder
copies the same helper. Existing offline OOBE delegates to it. No GUI, model,
Core/Policy/voice authority or other checkout was modified. The source fix does
not rebuild or certify the newer externally hosted draft image.

## Verification

- Regression reproduced before the fix: the real packaging tail included a
  synthetic builder private-key file and trust database in its tar archive.
- After the fix, whole/split packaging tests pass and the archive excludes both.
- Python WSL suite: 20 tests, 19 passed and 1 existing Windows file-symlink test
  skipped. Directory containment is exercised on Windows with a real junction.
- Initializer boundary tests cover root refusal, init-failure propagation and
  init-before-populate ordering. They use command doubles; they do not establish
  real pacman/GnuPG generation or deployment correctness.
- PowerShell installer contract check: simulated import calls the initializer as
  root, and failure stops before user setup. No WSL distro is created by the test.
- Bash syntax checks, PowerShell parse checks and `git diff --check` pass.
- Added the Python packaging suite to the existing CI workflow for Linux coverage.
- Independent code review found no actionable blocking security defect.

A complete Linux/WSL installation and real pacman identity generation were not run
on this host. No existing WSL distribution was stopped or changed. No hardware,
model accuracy, latency or deployment validation is claimed.

## Delivery and remote checks

Security commit `5759b62d9bc69e9bb3e507d45fda6f0ca4372ff0` was fast-forwarded to
GitHub main. A fresh remote verification confirmed all 13 removed objects/references
(nine assets, two releases and two tag names) return HTTP 404.

Actions run `40456703332` reports all three jobs failed **before starting**, with
empty step lists and no allocated runner. GitHub's failure annotation says recent
account payments failed or the spending limit needs increasing. Consequently no
Linux CI execution is claimed; account Billing & plans must be resolved before
retrying. The follow-up documentation-only receipt uses `[skip ci]` to avoid
queuing another identical billing-blocked run.
