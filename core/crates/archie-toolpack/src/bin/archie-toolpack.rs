//! `archie-toolpack` — install the tools Archie uses, from a signed catalog.
//!
//! ```text
//! archie-toolpack list     [--catalog FILE] [--keys FILE]
//! archie-toolpack list     --catalog-url HTTPS_URL [--keys FILE]
//! archie-toolpack install  TOOL [--catalog FILE | --catalog-url HTTPS_URL] [--keys FILE] [--root DIR] [--approve]
//! archie-toolpack rollback TOOL [--root DIR]
//! archie-toolpack status   [--root DIR]
//! archie-toolpack keygen   SECRET_FILE PUBLIC_FILE
//! archie-toolpack sign     CATALOG --secret-key FILE
//! ```
//!
//! Defaults: catalog `/etc/archie/toolpack/catalog.toml`, its signature beside it as
//! `catalog.toml.sig`, trusted keys `/etc/archie/toolpack/trusted-keys`, tool root
//! `/opt/archie/tools`. `install` goes through the same gate an agent does; `--approve`
//! stands for the person at the keyboard approving a tool that can reach the network or a
//! device.

use archie_policy::{Action, CapabilityRequest, Constraints, PolicyEngine, Principal, Scope};
use archie_toolpack::{
    download, open_catalog, sign, tool_resource, Catalog, InstallAdmission, InstallGate,
    InstallProposal, ToolRoot, TrustedKeys,
};
use archie_types::{Timestamp, TraceId};
use ed25519_dalek::SigningKey;
use futures_util::StreamExt;
use reqwest::{redirect::Policy, Client, Url};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const DEFAULT_CATALOG: &str = "/etc/archie/toolpack/catalog.toml";
const DEFAULT_KEYS: &str = "/etc/archie/toolpack/trusted-keys";
const DEFAULT_ROOT: &str = "/opt/archie/tools";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const CATALOG_LIMIT: usize = 1024 * 1024;
const SIGNATURE_LIMIT: usize = 130;

struct Args {
    words: Vec<String>,
}

impl Args {
    fn take(&mut self, flag: &str) -> Option<String> {
        let at = self.words.iter().position(|w| w == flag)?;
        if at + 1 >= self.words.len() {
            return None;
        }
        let value = self.words.remove(at + 1);
        self.words.remove(at);
        Some(value)
    }

    fn flag(&mut self, flag: &str) -> bool {
        match self.words.iter().position(|w| w == flag) {
            Some(at) => {
                self.words.remove(at);
                true
            }
            None => false,
        }
    }

    fn positional(&mut self, what: &str) -> Result<String, String> {
        if self.words.is_empty() {
            return Err(format!("missing {what}"));
        }
        Ok(self.words.remove(0))
    }
}

fn catalog_signature_url(value: &str) -> Result<Url, String> {
    let mut url = Url::parse(value).map_err(|_| "invalid catalog URL".to_string())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with(".toml")
    {
        return Err(
            "catalog URL must be an HTTPS .toml URL without credentials, query or fragment".into(),
        );
    }
    let signature_path = format!("{}.sig", url.path());
    url.set_path(&signature_path);
    Ok(url)
}

async fn fetch_bounded(client: &Client, url: &Url, limit: usize) -> Result<Vec<u8>, String> {
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if response.status().is_redirection() {
        return Err(format!(
            "{url}: redirects are not allowed for catalog metadata"
        ));
    }
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("{url}: response exceeds the {limit}-byte limit"));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("{url}: {e}"))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(format!("{url}: response exceeds the {limit}-byte limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn load_remote_catalog(url: &str, keys: &TrustedKeys) -> Result<Catalog, String> {
    let catalog_url = Url::parse(url).map_err(|_| "invalid catalog URL".to_string())?;
    let signature_url = catalog_signature_url(url)?;
    if catalog_url.host_str() != signature_url.host_str() {
        return Err("catalog signature must be adjacent to the catalog on the same host".into());
    }
    let client = Client::builder()
        .timeout(CATALOG_FETCH_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .redirect(Policy::none())
        .build()
        .map_err(|e| format!("cannot configure catalog HTTP client: {e}"))?;
    let bytes = fetch_bounded(&client, &catalog_url, CATALOG_LIMIT).await?;
    let signature = fetch_bounded(&client, &signature_url, SIGNATURE_LIMIT).await?;
    let signature_text = std::str::from_utf8(&signature)
        .map_err(|_| "catalog signature is not UTF-8".to_string())?;
    let trimmed = signature_text.trim();
    if trimmed.len() != 128 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("catalog signature must contain exactly 128 hexadecimal characters".into());
    }
    open_catalog(&bytes, trimmed, keys).map_err(|e| format!("{url}: {e}"))
}

async fn load_catalog(args: &mut Args) -> Result<Catalog, String> {
    let local_path = args.take("--catalog");
    let remote_url = args.take("--catalog-url");
    if local_path.is_some() && remote_url.is_some() {
        return Err("choose either --catalog or --catalog-url, not both".into());
    }
    let keys_path = args.take("--keys").unwrap_or_else(|| DEFAULT_KEYS.into());
    let keys_text = std::fs::read_to_string(&keys_path).map_err(|e| format!("{keys_path}: {e}"))?;
    let keys = TrustedKeys::parse(&keys_text).map_err(|e| format!("{keys_path}: {e}"))?;
    if let Some(url) = remote_url {
        return load_remote_catalog(&url, &keys).await;
    }
    let path = PathBuf::from(local_path.unwrap_or_else(|| DEFAULT_CATALOG.into()));
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let signature_path = path.with_file_name(format!(
        "{}.sig",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("catalog.toml")
    ));
    let signature = std::fs::read_to_string(&signature_path)
        .map_err(|e| format!("{}: {e}", signature_path.display()))?;
    open_catalog(&bytes, &signature, &keys).map_err(|e| format!("{}: {e}", path.display()))
}

async fn list(mut args: Args) -> Result<ExitCode, String> {
    let catalog = load_catalog(&mut args).await?;
    println!("{} (signature verified)", catalog.name);
    for tool in &catalog.tools {
        println!(
            "  {:<16} {:<12} {:<16} {} MiB  {}",
            tool.id,
            tool.version,
            tool.risk.as_str(),
            tool.artifact.size.div_ceil(1024 * 1024),
            tool.title
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn install(mut args: Args) -> Result<ExitCode, String> {
    let id = args.positional("the tool to install")?;
    let root = ToolRoot::new(args.take("--root").unwrap_or_else(|| DEFAULT_ROOT.into()));
    let approve = args.flag("--approve");
    let catalog = load_catalog(&mut args).await?;

    let principal = Principal::Human {
        id: "toolpack-cli".into(),
        authenticated: true,
    };
    let operation = (Timestamp::now().as_millis() as u64).max(1);
    let lease = PolicyEngine::default()
        .decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource: tool_resource(root.path(), &id),
            scope: Scope::OneOperation { operation },
            constraints: Constraints::with_actions([Action::Write]),
            justification: format!("install {id} from the toolpack"),
        })
        .lease
        .ok_or("policy did not grant a lease to install this tool")?;
    let proposal = InstallProposal {
        tool_id: id.clone(),
        operation_id: operation,
        task_id: None,
    };
    let authorized = match InstallGate::authorize(
        &catalog,
        root.path(),
        &lease,
        &principal,
        true,
        &proposal,
        Timestamp::now(),
    ) {
        InstallAdmission::Allowed(install) => install,
        InstallAdmission::NeedsApproval { install, reason } if approve => {
            eprintln!("approved by the operator: {reason}");
            install
        }
        InstallAdmission::NeedsApproval { reason, .. } => {
            eprintln!("needs approval ({reason}); rerun with --approve if you are the person approving it");
            return Ok(ExitCode::from(3));
        }
        InstallAdmission::Refused(refusal) => {
            eprintln!("refused ({}): {refusal}", refusal.code());
            return Ok(ExitCode::from(2));
        }
    };

    let artifact = authorized.artifact();
    let download_path = root.download_path(&authorized);
    if let Some(parent) = download_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    println!(
        "downloading {} {} ({} bytes)",
        id,
        authorized.tool().version,
        artifact.size
    );
    if let Err(error) = download(&authorized, &download_path, DOWNLOAD_TIMEOUT).await {
        eprintln!("download failed ({}): {error}", error.code());
        return Ok(ExitCode::from(4));
    }
    println!("verified sha256 {}", artifact.sha256);
    let result = root.install(&authorized, &download_path);
    let _ = std::fs::remove_file(&download_path);
    let installed = result.map_err(|e| e.to_string())?;
    println!(
        "installed {} {}{}",
        installed.id,
        installed.version,
        installed
            .previous
            .map(|p| format!(" (previous: {p})"))
            .unwrap_or_default()
    );
    for binary in installed.binaries {
        println!("  {}", binary.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn rollback(mut args: Args) -> Result<ExitCode, String> {
    let id = args.positional("the tool to roll back")?;
    let root = ToolRoot::new(args.take("--root").unwrap_or_else(|| DEFAULT_ROOT.into()));
    let version = root.rollback(&id).map_err(|e| e.to_string())?;
    println!("{id} is back on {version}");
    Ok(ExitCode::SUCCESS)
}

fn status(mut args: Args) -> Result<ExitCode, String> {
    let root = ToolRoot::new(args.take("--root").unwrap_or_else(|| DEFAULT_ROOT.into()));
    for id in root.tools() {
        println!(
            "{:<16} current {:<12} previous {}",
            id,
            root.current(&id).unwrap_or_else(|| "-".into()),
            root.previous(&id).unwrap_or_else(|| "-".into())
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(path)
        .and_then(|mut file| file.write_all(contents.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn keygen(mut args: Args) -> Result<ExitCode, String> {
    let secret = PathBuf::from(args.positional("the secret key file")?);
    let public = PathBuf::from(args.positional("the public key file")?);
    let key = SigningKey::generate(&mut rand::rngs::OsRng);
    write_private(&secret, &format!("{}\n", hex::encode(key.to_bytes())))?;
    std::fs::write(
        &public,
        format!("{}\n", hex::encode(key.verifying_key().to_bytes())),
    )
    .map_err(|e| format!("{}: {e}", public.display()))?;
    println!(
        "wrote {} (keep it off the node) and {}",
        secret.display(),
        public.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn sign_catalog(mut args: Args) -> Result<ExitCode, String> {
    let catalog_path = PathBuf::from(args.positional("the catalog to sign")?);
    let secret_path = args.take("--secret-key").ok_or("sign needs --secret-key")?;
    let bytes =
        std::fs::read(&catalog_path).map_err(|e| format!("{}: {e}", catalog_path.display()))?;
    // Never sign something that would not install.
    let text = std::str::from_utf8(&bytes).map_err(|_| "the catalog is not UTF-8".to_string())?;
    Catalog::parse(text).map_err(|e| format!("{}: {e}", catalog_path.display()))?;
    let secret_text =
        std::fs::read_to_string(&secret_path).map_err(|e| format!("{secret_path}: {e}"))?;
    let secret: [u8; 32] = hex::decode(secret_text.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| format!("{secret_path}: not a 32-byte hex key"))?;
    let signature = sign(&bytes, &SigningKey::from_bytes(&secret));
    let signature_path = PathBuf::from(format!("{}.sig", catalog_path.display()));
    std::fs::write(&signature_path, format!("{signature}\n"))
        .map_err(|e| format!("{}: {e}", signature_path.display()))?;
    println!("wrote {}", signature_path.display());
    Ok(ExitCode::SUCCESS)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let mut words: Vec<String> = std::env::args().skip(1).collect();
    let command = if words.is_empty() {
        String::new()
    } else {
        words.remove(0)
    };
    let args = Args { words };
    let result = match command.as_str() {
        "list" => list(args).await,
        "install" => install(args).await,
        "rollback" => rollback(args),
        "status" => status(args),
        "keygen" => keygen(args),
        "sign" => sign_catalog(args),
        _ => Err("usage: archie-toolpack list|install|rollback|status|keygen|sign …".into()),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("archie-toolpack: {message}");
            ExitCode::FAILURE
        }
    }
}
