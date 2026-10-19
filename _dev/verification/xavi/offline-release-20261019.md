# Offline WSL draft build

Owner selected one self-contained `.wsl` hosted externally, with a GitHub draft
release linking to it. The draft is416918011, tag
`wsl-bonsai2-9b-pr41-2026.10.19`; it is not published. No existing release is replaced.

## Inputs and boundaries

- Design baseline5c86b46, including Xavi coauthorship and main917372596 integration.
- Clean legacy operator image1046675202bytes, SHA-256
  `50c123adeb5134c6d6b7b898eae060d774435e985c49f8dec581bb7dc0d9eab7`.
  It was downloaded and checked, never exported from the running personal node.
- Current source development Rust binaries, debug information stripped only in the
  staging tree; current source native renderer. Build jobs bounded to2.
- Five model files total4280574324bytes: exact native Bonsai2 9B PTQ1_0 base,
  Nemotron3.5 ASR q8, Spanish/English Piper, IRIS Beta2.1 step132000.
  Ten public encoder-cache files bring the hashed payload to5816954444bytes.
  The image also contains dependencies, model attribution and runtime licenses.
- PostgreSQL18.6 with reviewed TimescaleDB2.29.2 and pgvector0.8.6. Additional
  numactl, Qt Quick3D, QuickTimeline, ShaderTools, OpenXR and jsoncpp runtime files
  come from the same installed package versions as the clean base's Qt/glibc ABI.
- Per-image payload JSON contains model digests, source revision and binary hashes.
  Image assembly helpers and build logs are retained in the local handoff directory.
  `scripts/wsl/package-offline.sh` validates staged integrity and refuses builder
  keys, personal state, unfinished source IDs, existing outputs and in-tree outputs.

## Validation actually performed

- Python packaging tests:7 pass on Linux;4 pass/3 platform skips on Windows. They use
  tiny files and establish packaging behavior only, never model quality/hardware.
  Stubbed OOBE boundary tests prove a runtime/port failure preserves installation,
  while an invalid payload still fails OOBE; they do not certify Windows installation.
- Voice library:51 pass. New CLI timeout test passes without default features and
  with native speech. Default90s preserved; explicit waits limited to1..900s.
- Current source builds completed, including native speech and packaged CLI tools.
- Inside the staged image, dependencies of nine primary executables resolve.
  Independent review caught missing numactl/Qt Quick3D; these were added.
- A real fresh Unix-socket-only PostgreSQL cluster initialized and loaded exact
  TimescaleDB2.29.2/pgvector0.8.6 inside the image. It was stopped and removed;
  temporary device nodes were removed. No live database was used for that proof.
- Staged model integrity checked inside chroot:15 files matched their digests.
- Real full source voice prompt against the existing owned CPU Bonsai server:
  `Responde solamente: listo.` produced `listo`, first words257882ms, complete257883ms.
  This is one smoke observation, not a latency or accuracy benchmark.
- Voice/Orb socket propagation and Core/displayd readiness corrected after independent
  review. Native profile selection requires actual ACK before startup reports success.

## Release gates and limitations

Compression/output checksum, archive inspection, clean WSL import and external upload
must be recorded separately when completed. Modern double-click installation remains
unverified because this host is WSL2.2.4; updating it would interrupt other active work.
Do not mark this as a tested modern installer or complete GPU/IRIS/Stage certification.

The generated pacman private key inherited from the old base is removed from staging,
as are random seeds and any staged SSH host private keys. OOBE creates a machine-local
package-manager key. No account token, real node state or training data is copied.

The other HF account was confirmed as PentaVM, but its app OAuth scopes still permit
read-repos/read-mcp/jobs only. No CLI write credential was present at inspection.
External upload requires genuine write authentication; never reuse or extract the app
OAuth token as a workaround. Keep external storage private while the release is a draft.
