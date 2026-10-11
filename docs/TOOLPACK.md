# The toolpack

The toolpack is how Archie gets software a task needs without a person running an
installer: a document renderer, a PDF tool, a solver. It fetches the tool from a
published release, checks it, and puts it in place, from a catalog Archie can read and a
person can review. `core/crates/archie-toolpack` is the mechanism, and `archie-toolpack`
is its command line.

It is not a package manager. System packages still come from `pacman`, with
administrative approval. A tool in the toolpack runs only through the executor, inside a
workspace its task was granted, and never as root.

## What it trusts, and when

```text
catalog.toml + catalog.toml.sig
  │  Ed25519 signature over "archie-toolpack/1\0" + the file, by a key in trusted-keys,
  │  checked before a byte of the catalog is parsed
  ▼
Catalog ── URLs HTTPS without credentials (or http to 127.0.0.1 for a local mirror),
  │        64-hex SHA-256, exact size ≤ 2 GiB, binary paths relative and inside the tool,
  │        no system_admin tools, no unknown fields
  ▼
InstallGate ── a one-operation Write lease on <root>/<id>
  │           └ read_only, workspace_write: installs on the lease
  │           └ network_egress, device_access: waits for a person's approval
  │  AuthorizedInstall   (no public constructor)
  ▼
download ── size enforced while streaming, SHA-256 at the end, at most 5 redirects and
  │         only to HTTPS hosts the catalog lists; a failed download leaves no file
  ▼
ToolRoot::install ── unpack in .staging: no "..", no absolute paths, no symlinks or hard
                     links, no overwriting, no setuid; entries and bytes capped. Then one
                     rename into <id>/<version>, one rename of <id>/current. Rollback
                     swaps current and previous.
```

**The signature vouches for the bytes, the checks for their sense.** A correctly signed
catalog that declares a zero-byte artifact or a binary at `../x` is still refused.

**The digest is what decides.** A redirect host list only narrows where bytes may come
from. The SHA-256 is what makes the bytes the ones the catalog's signer saw.

**An install changes nothing until it has fully succeeded.** A version directory appears
in a single rename, complete or not at all. A running program keeps the version it
started with.

## Catalog format

```toml
version = 1
name = "ArchieOS tools"
# Where release downloads may be redirected, e.g. the object store behind a release page.
redirect_hosts = ["release-assets.githubusercontent.com"]

[[tool]]
id = "ripgrep"
title = "ripgrep"
version = "14.1.1"
license = "MIT OR Unlicense"
risk = "read_only"                  # read_only | workspace_write | network_egress | device_access
capabilities = ["workspace.search"]

[tool.artifact]
url = "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1-x86_64-unknown-linux-musl.tar.gz"
sha256 = "…64 lowercase hex characters…"
size = 2600000
format = "tar_gz"                   # tar_gz | binary
binaries = ["ripgrep-14.1.1-x86_64-unknown-linux-musl/rg"]
```

The digest and size above are placeholders, and the size is a guess. A real entry is
written from the release itself: the digest and size come from the file that was
downloaded and inspected, not from anyone's memory.

## Using it

On a release machine, never on a node:

```bash
archie-toolpack keygen toolpack-release.key toolpack-release.pub
```

```bash
archie-toolpack sign catalog.toml --secret-key toolpack-release.key
```

`sign` refuses a catalog that would not validate. On a node, the public key goes into
`/etc/archie/toolpack/trusted-keys`, one hex key per line. Then:

```bash
archie-toolpack list
```

```bash
archie-toolpack install typst
```

```bash
archie-toolpack status
```

```bash
archie-toolpack rollback typst
```

`install` exits 0 installed, 2 refused by the gate, 3 needs approval (`--approve` stands for
the person at the keyboard), 4 the download did not verify. Tools land under
`/opt/archie/tools/<id>/<version>/`, and `<id>/current` names the one in use.

## Remote catalog

The default catalog is local. The list and install commands also accept an explicit
remote catalog URL:

```bash
archie-toolpack list --catalog-url https://archieos-hub.vercel.app/registry/v1/toolpack/catalog.toml
archie-toolpack install ripgrep --catalog-url https://archieos-hub.vercel.app/registry/v1/toolpack/catalog.toml
```

The client fetches the adjacent signature over HTTPS, refuses redirects, bounds the
TOML to 1 MiB and the signature response to 130 bytes (128 hexadecimal characters plus
a line ending). It verifies the signature against /etc/archie/toolpack/trusted-keys
before parsing the TOML or considering a download. The existing install gate,
capability checks, size limits and SHA-256 verification still apply. The local-file and
remote URL options are mutually exclusive; omitting both keeps the local default.

Remote catalogs remain opt-in. The Hub index advertises no Toolpack URL until a
maintainer has reviewed and signed a catalog and the corresponding public key is
installed in the image's trusted-key file. The signing private key stays offline;
neither it nor a generated replacement belongs in the node, source repository, GitHub
or Vercel.

## Tested, and how

`cargo test -p archie-toolpack`. Each rule has a test built as the attack or failure it
guards against:

- Catalogs with plain http, credentials in the URL, `file://`, uppercase or short digests,
  zero or oversized sizes, binaries at `../evil`, `/usr/bin/x`, `a\b` or `./x`, a
  `system_admin` tool, an unknown field, a duplicate id.
- A signature that fails after one changed byte, by an untrusted key, over the file without
  its domain tag, and with no trusted keys at all.
- Archives with `../evil`, an absolute path, a symlink, a hard link, a file repeated to
  overwrite itself, a setuid bit, too many entries and too many bytes.
- Installs of two versions, a double rollback, and an artifact missing its binary that
  leaves the current version and the staging directory as they were.
- Downloads from a loopback server that sends a wrong digest, more bytes than declared,
  a wrong `Content-Length`, a redirect to an unlisted host and a 404. A test also checks
  that an existing file at the destination survives the failure.

## Not done

- **Only `.tar.gz` and single binaries.** Many releases ship `.tar.xz`, `.tar.zst` or
  `.zip`. Adding them means pure-Rust decoders, or linking liblzma and libzstd, and each
  gets the same entry rules.
- **No signed catalog is published, and no release repositories exist.** Creating
  repositories and publishing releases under the project's name is for the maintainers to decide and do.
  The first catalog's candidates are the automatable tools in
  [research/PRELOADED-TOOLS.md](research/PRELOADED-TOOLS.md) that ship static release
  binaries.
- **Registration is manual.** An installed tool's capabilities do not yet reach
  `archie-registry`, and the executor does not yet resolve binaries through `ToolRoot`.
- **No revocation.** A catalog cannot yet withdraw a version it once signed. Key rotation is
  replacing `trusted-keys`.
- **The download goes straight to the source.** It is not yet routed through the node's
  network capability, as web search is.
