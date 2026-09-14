//! Catalog, signature, gate, unpacking and installation, each against the way it would be
//! attacked or would fail.

use archie_policy::{Action, CapabilityRequest, Constraints, Lease, PolicyEngine, Principal, Scope};
use archie_toolpack::{
    open_catalog, sign, tool_resource, unpack_tar_gz, AuthorizedInstall, Catalog, CatalogError, InstallAdmission,
    InstallError, InstallGate, InstallProposal, InstallRefusal, Limits, OpenError, SignatureError, ToolRoot,
    TrustedKeys, UnpackError,
};
use archie_types::{AgentId, Timestamp, TraceId};
use ed25519_dalek::{Signer, SigningKey};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tar::{Builder, EntryType, Header};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-toolpack-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Entry<'a> {
    risk: &'a str,
    version: &'a str,
    url: &'a str,
    sha256: String,
    size: u64,
    format: &'a str,
    binaries: &'a str,
}

impl Default for Entry<'_> {
    fn default() -> Self {
        Self {
            risk: "workspace_write",
            version: "1.2.0",
            url: "https://releases.example.org/rendertool-1.2.0.tar.gz",
            sha256: "a".repeat(64),
            size: 1024,
            format: "tar_gz",
            binaries: r#""rendertool/bin/render""#,
        }
    }
}

fn catalog_text(entry: &Entry) -> String {
    format!(
        r#"version = 1
name = "test tools"
redirect_hosts = ["objects.example.org"]

[[tool]]
id = "rendertool"
title = "A renderer"
version = "{version}"
license = "Apache-2.0"
risk = "{risk}"
capabilities = ["document.render"]

[tool.artifact]
url = "{url}"
sha256 = "{sha256}"
size = {size}
format = "{format}"
binaries = [{binaries}]
"#,
        version = entry.version,
        risk = entry.risk,
        url = entry.url,
        sha256 = entry.sha256,
        size = entry.size,
        format = entry.format,
        binaries = entry.binaries,
    )
}

fn lease_for(root: &Path, id: &str, principal: &Principal, operation: u64) -> Lease {
    PolicyEngine::default()
        .decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource: tool_resource(root, id),
            scope: Scope::OneOperation { operation },
            constraints: Constraints::with_actions([Action::Write]),
            justification: "install".into(),
        })
        .lease
        .unwrap()
}

fn agent() -> Principal {
    Principal::Agent {
        agent: AgentId::new(),
    }
}

fn authorize(catalog: &Catalog, root: &Path, operation: u64) -> InstallAdmission {
    let principal = agent();
    let lease = lease_for(root, "rendertool", &principal, operation);
    InstallGate::authorize(
        catalog,
        root,
        &lease,
        &principal,
        false,
        &InstallProposal {
            tool_id: "rendertool".into(),
            operation_id: operation,
            task_id: None,
        },
        Timestamp::now(),
    )
}

fn allowed(catalog: &Catalog, root: &Path, operation: u64) -> AuthorizedInstall {
    match authorize(catalog, root, operation) {
        InstallAdmission::Allowed(install) => install,
        other => panic!("expected an allowed install, got {other:?}"),
    }
}

/// A tar.gz of `(path, contents, mode)` files, paths written into the header byte for byte.
fn tarball(path: &Path, files: &[(&[u8], &[u8], u32)]) {
    let mut builder = Builder::new(GzEncoder::new(std::fs::File::create(path).unwrap(), Compression::fast()));
    for (name, contents, mode) in files {
        let mut header = Header::new_gnu();
        header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
        header.set_size(contents.len() as u64);
        header.set_mode(*mode);
        header.set_entry_type(EntryType::Regular);
        header.set_cksum();
        builder.append(&header, *contents).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

fn digest(path: &Path) -> (String, u64) {
    let bytes = std::fs::read(path).unwrap();
    (hex::encode(Sha256::digest(&bytes)), bytes.len() as u64)
}

#[test]
fn a_catalog_is_checked_like_input_from_outside() {
    assert!(Catalog::parse(&catalog_text(&Entry::default())).is_ok());
    let loopback = Entry {
        url: "http://127.0.0.1:8080/rendertool.tar.gz",
        ..Entry::default()
    };
    assert!(Catalog::parse(&catalog_text(&loopback)).is_ok(), "a mirror on this machine may be plain http");

    let refused = |entry: Entry| Catalog::parse(&catalog_text(&entry)).unwrap_err();
    for url in [
        "http://releases.example.org/x.tar.gz",
        "https://user:pass@releases.example.org/x.tar.gz",
        "ftp://releases.example.org/x.tar.gz",
        "file:///tmp/x.tar.gz",
        "not a url",
    ] {
        assert!(matches!(refused(Entry { url, ..Entry::default() }), CatalogError::Url(_)), "{url}");
    }
    assert!(matches!(refused(Entry { sha256: "A".repeat(64), ..Entry::default() }), CatalogError::Digest(_)));
    assert!(matches!(refused(Entry { sha256: "a".repeat(63), ..Entry::default() }), CatalogError::Digest(_)));
    assert!(matches!(refused(Entry { size: 0, ..Entry::default() }), CatalogError::Size(_)));
    assert!(matches!(refused(Entry { size: 3 << 30, ..Entry::default() }), CatalogError::Size(_)));
    for binaries in [r#""../evil""#, r#""/usr/bin/render""#, r#""a\\b""#, r#""./render""#, ""] {
        assert!(matches!(refused(Entry { binaries, ..Entry::default() }), CatalogError::Binary(..)), "{binaries}");
    }
    assert!(matches!(
        refused(Entry { format: "binary", binaries: r#""bin/render""#, ..Entry::default() }),
        CatalogError::BinaryArtifact(_)
    ));
    assert!(matches!(refused(Entry { risk: "system_admin", ..Entry::default() }), CatalogError::SystemAdmin(_)));
    assert!(matches!(refused(Entry { version: "../1", ..Entry::default() }), CatalogError::VersionString(..)));

    let text = catalog_text(&Entry::default());
    let unknown_field = text.replace("license = ", "homepage = \"x\"\nlicense = ");
    assert!(matches!(Catalog::parse(&unknown_field), Err(CatalogError::Syntax(_))));
    assert!(matches!(Catalog::parse(&text.replace("version = 1\n", "version = 2\n")), Err(CatalogError::Version(2))));
    assert!(matches!(Catalog::parse(&text.replace("id = \"rendertool\"", "id = \"Render\"")), Err(CatalogError::Id(_))));
    assert!(matches!(
        Catalog::parse(&text.replace("document.render", "Document")),
        Err(CatalogError::Capability(..))
    ));
    assert!(matches!(
        Catalog::parse(&text.replace("objects.example.org", "https://objects.example.org")),
        Err(CatalogError::RedirectHost(_))
    ));
    let twice = format!("{text}{}", &text[text.find("[[tool]]").unwrap()..]);
    assert!(matches!(Catalog::parse(&twice), Err(CatalogError::Duplicate(_))));
}

#[test]
fn only_a_trusted_signature_opens_a_catalog() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let keys = TrustedKeys::from_keys(vec![key.verifying_key()]);
    let bytes = catalog_text(&Entry::default()).into_bytes();
    let signature = sign(&bytes, &key);
    assert!(open_catalog(&bytes, &signature, &keys).is_ok());

    let mut tampered = bytes.clone();
    let at = tampered.iter().position(|b| *b == b'2').unwrap();
    tampered[at] = b'9';
    assert_eq!(open_catalog(&tampered, &signature, &keys).unwrap_err(), OpenError::Signature(SignatureError::Untrusted));

    let stranger = SigningKey::from_bytes(&[8; 32]);
    assert_eq!(keys.verify(&bytes, &sign(&bytes, &stranger)), Err(SignatureError::Untrusted));
    assert_eq!(
        keys.verify(&bytes, &hex::encode(key.sign(&bytes).to_bytes())),
        Err(SignatureError::Untrusted),
        "a signature over the bare bytes, made for some other purpose, is not a catalog signature"
    );
    assert_eq!(TrustedKeys::from_keys(vec![]).verify(&bytes, &signature), Err(SignatureError::NoKeys));
    assert_eq!(keys.verify(&bytes, "zz"), Err(SignatureError::Malformed));

    let listed = format!("# release key\n\n{}\n", hex::encode(key.verifying_key().to_bytes()));
    assert!(TrustedKeys::parse(&listed).unwrap().verify(&bytes, &signature).is_ok());
    assert_eq!(TrustedKeys::parse("# ok\nnot-a-key\n").unwrap_err(), SignatureError::Key(2));

    // Signed, and still invalid: the signature vouches for the bytes, not for their sense.
    let invalid = catalog_text(&Entry { size: 0, ..Entry::default() }).into_bytes();
    assert!(matches!(
        open_catalog(&invalid, &sign(&invalid, &key), &keys),
        Err(OpenError::Catalog(CatalogError::Size(_)))
    ));
}

#[test]
fn the_gate_decides_by_lease_and_risk() {
    let root = scratch("gate");
    let workspace = Catalog::parse(&catalog_text(&Entry::default())).unwrap();
    assert!(matches!(authorize(&workspace, &root, 11), InstallAdmission::Allowed(_)));

    let network = Catalog::parse(&catalog_text(&Entry { risk: "network_egress", ..Entry::default() })).unwrap();
    assert!(matches!(authorize(&network, &root, 12), InstallAdmission::NeedsApproval { .. }));
    let devices = Catalog::parse(&catalog_text(&Entry { risk: "device_access", ..Entry::default() })).unwrap();
    assert!(matches!(authorize(&devices, &root, 13), InstallAdmission::NeedsApproval { .. }));

    let principal = agent();
    let other_tool = lease_for(&root, "sometool", &principal, 14);
    let proposal = InstallProposal {
        tool_id: "rendertool".into(),
        operation_id: 14,
        task_id: None,
    };
    match InstallGate::authorize(&workspace, &root, &other_tool, &principal, false, &proposal, Timestamp::now()) {
        InstallAdmission::Refused(InstallRefusal::Lease(_)) => {}
        other => panic!("a lease for another tool must not install this one: {other:?}"),
    }
    let unknown = InstallProposal {
        tool_id: "missing".into(),
        ..proposal.clone()
    };
    assert!(matches!(
        InstallGate::authorize(&workspace, &root, &other_tool, &principal, false, &unknown, Timestamp::now()),
        InstallAdmission::Refused(InstallRefusal::UnknownTool(_))
    ));
    let no_operation = InstallProposal {
        operation_id: 0,
        ..proposal
    };
    assert!(matches!(
        InstallGate::authorize(&workspace, &root, &other_tool, &principal, false, &no_operation, Timestamp::now()),
        InstallAdmission::Refused(InstallRefusal::NoOperation)
    ));
}

fn raw_tarball(path: &Path, name: &[u8], kind: EntryType, link: Option<&str>) {
    let mut builder = Builder::new(GzEncoder::new(std::fs::File::create(path).unwrap(), Compression::fast()));
    let mut header = Header::new_gnu();
    header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
    header.set_entry_type(kind);
    header.set_mode(0o644);
    let contents: &[u8] = if kind == EntryType::Regular { b"payload" } else { b"" };
    header.set_size(contents.len() as u64);
    if let Some(link) = link {
        // Written raw: the builder would refuse to create the entry this test is about.
        header.as_gnu_mut().unwrap().linkname[..link.len()].copy_from_slice(link.as_bytes());
    }
    header.set_cksum();
    builder.append(&header, contents).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

#[test]
fn unpacking_refuses_paths_and_entries_that_escape() {
    let dir = scratch("unpack");
    let dest = dir.join("dest");

    let escape = dir.join("escape.tar.gz");
    raw_tarball(&escape, b"../evil", EntryType::Regular, None);
    assert!(matches!(unpack_tar_gz(&escape, &dest, Limits::default()), Err(UnpackError::UnsafePath(_))));
    assert!(!dir.join("evil").exists(), "nothing was written outside");

    let absolute = dir.join("absolute.tar.gz");
    raw_tarball(&absolute, b"/tmp/archie-toolpack-absolute", EntryType::Regular, None);
    assert!(matches!(unpack_tar_gz(&absolute, &dest, Limits::default()), Err(UnpackError::UnsafePath(_))));

    let symlink = dir.join("symlink.tar.gz");
    raw_tarball(&symlink, b"bin", EntryType::Symlink, Some("/etc"));
    assert!(matches!(
        unpack_tar_gz(&symlink, &dest, Limits::default()),
        Err(UnpackError::UnsupportedEntry { .. })
    ));
    let hardlink = dir.join("hardlink.tar.gz");
    raw_tarball(&hardlink, b"passwd", EntryType::Link, Some("/etc/passwd"));
    assert!(matches!(
        unpack_tar_gz(&hardlink, &dest, Limits::default()),
        Err(UnpackError::UnsupportedEntry { .. })
    ));

    let good = dir.join("good.tar.gz");
    tarball(&good, &[(b"tool/bin/run", b"#!/bin/sh\n", 0o4755), (b"tool/README", b"hello", 0o666)]);
    let fresh = dir.join("fresh");
    let unpacked = unpack_tar_gz(&good, &fresh, Limits::default()).unwrap();
    assert_eq!(unpacked.files, 2);
    assert_eq!(std::fs::read(fresh.join("tool/README")).unwrap(), b"hello");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(fresh.join("tool/bin/run")).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o755, "setuid is dropped, execute is kept");
        let mode = std::fs::metadata(fresh.join("tool/README")).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o644, "world-writable is dropped");
    }

    let duplicate = dir.join("duplicate.tar.gz");
    tarball(&duplicate, &[(b"x", b"first", 0o644), (b"x", b"second", 0o644)]);
    assert!(
        unpack_tar_gz(&duplicate, &dir.join("dup"), Limits::default()).is_err(),
        "an archive cannot overwrite a file, not even its own"
    );

    assert!(matches!(
        unpack_tar_gz(&good, &dir.join("few"), Limits { max_entries: 1, max_bytes: 1 << 20 }),
        Err(UnpackError::TooManyEntries(1))
    ));
    assert!(matches!(
        unpack_tar_gz(&good, &dir.join("small"), Limits { max_entries: 10, max_bytes: 8 }),
        Err(UnpackError::TooLarge(8))
    ));
}

#[test]
fn installs_are_atomic_and_roll_back() {
    let dir = scratch("install");
    let root = ToolRoot::new(dir.join("tools"));

    let install_version = |version: &str, files: &[(&[u8], &[u8], u32)], operation: u64| {
        let archive = dir.join(format!("rendertool-{version}.tar.gz"));
        tarball(&archive, files);
        let (sha256, size) = digest(&archive);
        let catalog = Catalog::parse(&catalog_text(&Entry {
            version,
            sha256,
            size,
            ..Entry::default()
        }))
        .unwrap();
        let authorized = allowed(&catalog, root.path(), operation);
        root.install(&authorized, &archive)
    };
    let render: &[(&[u8], &[u8], u32)] = &[(b"rendertool/bin/render", b"v", 0o755)];

    let first = install_version("1.2.0", render, 21).unwrap();
    assert_eq!(first.previous, None);
    assert_eq!(root.current("rendertool").as_deref(), Some("1.2.0"));
    assert!(root.binary("rendertool", "rendertool/bin/render").is_some());

    let second = install_version("1.3.0", render, 22).unwrap();
    assert_eq!(second.previous.as_deref(), Some("1.2.0"));
    assert_eq!(root.current("rendertool").as_deref(), Some("1.3.0"));

    assert_eq!(root.rollback("rendertool").unwrap(), "1.2.0");
    assert_eq!(root.current("rendertool").as_deref(), Some("1.2.0"));
    assert_eq!(root.rollback("rendertool").unwrap(), "1.3.0", "a second rollback undoes the first");

    // An artifact without the binary it promises changes nothing.
    let broken = install_version("1.4.0", &[(b"rendertool/README", b"no binary here", 0o644)], 23);
    assert!(matches!(broken, Err(InstallError::MissingBinary(_))));
    assert_eq!(root.current("rendertool").as_deref(), Some("1.3.0"));
    assert!(!root.path().join("rendertool/1.4.0").exists());
    let staging: Vec<_> = std::fs::read_dir(root.path().join(".staging")).unwrap().collect();
    assert!(staging.is_empty(), "a failed install leaves no staging behind");

    // Installing a version that is already there just points at it again.
    let again = install_version("1.2.0", render, 24).unwrap();
    assert_eq!(again.previous.as_deref(), Some("1.3.0"));
    assert_eq!(root.tools(), vec!["rendertool".to_string()]);

    // A single-binary artifact.
    let plain = dir.join("plain-binary");
    std::fs::write(&plain, b"\x7fELF...").unwrap();
    let (sha256, size) = digest(&plain);
    let catalog = Catalog::parse(&catalog_text(&Entry {
        version: "2.0.0",
        sha256,
        size,
        format: "binary",
        binaries: r#""render""#,
        ..Entry::default()
    }))
    .unwrap();
    root.install(&allowed(&catalog, root.path(), 25), &plain).unwrap();
    assert!(root.binary("rendertool", "render").is_some());

    let fresh = ToolRoot::new(dir.join("empty"));
    assert!(matches!(fresh.rollback("rendertool"), Err(InstallError::NothingToRollBack(_))));
}
