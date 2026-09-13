//! `archie-search` — one web search through the same gate Cerebra uses.
//!
//! A smoke path, like `archie-infer`. The query goes through [`SearchGate`] and
//! [`TavilyClient`] exactly as it would from the loop. The key is read from
//! `TAVILY_API_KEY`, sealed into a vault that exists only inside this process, and
//! brokered back out for the one request, so the client receives the same kind of
//! credential handle it gets in the daemon rather than a string passed beside it.

use archie_search::{
    SearchAdmission, SearchAuthority, SearchDepth, SearchGate, SearchOptions, SearchProposal,
    SearchProviderManifest, SearchRegistry, SearchResults, SearchTopic, TavilyClient, TimeRange,
};
use archie_types::capability::well_known;
use archie_types::{DataRetention, PrivacyGrade};
use archie_vault::{EntryClass, SealedCredential, Vault, VaultKey, VaultRef};
use clap::{Parser, ValueEnum};
use ring::rand::{SecureRandom, SystemRandom};
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, Zeroizing};

const KEY_ENV: &str = "TAVILY_API_KEY";
const DEFAULT_CREDENTIAL: &str = "vault:tavily/main";
const ACTOR: &str = "archie-search-cli";
const SNIPPET_PREVIEW_CHARS: usize = 400;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DepthArg {
    UltraFast,
    Fast,
    Basic,
    Advanced,
}

impl From<DepthArg> for SearchDepth {
    fn from(value: DepthArg) -> Self {
        match value {
            DepthArg::UltraFast => Self::UltraFast,
            DepthArg::Fast => Self::Fast,
            DepthArg::Basic => Self::Basic,
            DepthArg::Advanced => Self::Advanced,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TopicArg {
    General,
    News,
    Finance,
}

impl From<TopicArg> for SearchTopic {
    fn from(value: TopicArg) -> Self {
        match value {
            TopicArg::General => Self::General,
            TopicArg::News => Self::News,
            TopicArg::Finance => Self::Finance,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TimeRangeArg {
    Day,
    Week,
    Month,
    Year,
}

impl From<TimeRangeArg> for TimeRange {
    fn from(value: TimeRangeArg) -> Self {
        match value {
            TimeRangeArg::Day => Self::Day,
            TimeRangeArg::Week => Self::Week,
            TimeRangeArg::Month => Self::Month,
            TimeRangeArg::Year => Self::Year,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PrivacyArg {
    Public,
    Internal,
    Sensitive,
    Restricted,
}

impl From<PrivacyArg> for PrivacyGrade {
    fn from(value: PrivacyArg) -> Self {
        match value {
            PrivacyArg::Public => Self::Public,
            PrivacyArg::Internal => Self::Internal,
            PrivacyArg::Sensitive => Self::Sensitive,
            PrivacyArg::Restricted => Self::Restricted,
        }
    }
}

#[derive(Parser, Debug)]
#[command(about = "Policy-gated web search through Tavily")]
struct Args {
    /// The search query. Read from stdin when omitted.
    query: Option<String>,
    #[arg(long, default_value_t = 5)]
    max_results: u8,
    #[arg(long, value_enum, default_value = "basic")]
    depth: DepthArg,
    #[arg(long, value_enum, default_value = "general")]
    topic: TopicArg,
    #[arg(long, value_enum)]
    time_range: Option<TimeRangeArg>,
    /// Only return results from this domain. Repeatable.
    #[arg(long = "include-domain", value_name = "DOMAIN")]
    include_domains: Vec<String>,
    /// Never return results from this domain. Repeatable.
    #[arg(long = "exclude-domain", value_name = "DOMAIN")]
    exclude_domains: Vec<String>,
    /// How sensitive the query is. Above public, the query is refused unless zero data
    /// retention has been verified for the account.
    #[arg(long, value_enum, default_value = "public")]
    privacy: PrivacyArg,
    /// Assert that zero data retention has been verified for this Tavily account.
    #[arg(long)]
    zero_retention_verified: bool,
    /// Permit advanced depth, which costs two credits per query.
    #[arg(long)]
    allow_advanced: bool,
    /// A provider manifest to use instead of Tavily's built-in defaults.
    #[arg(long, value_name = "FILE")]
    manifest: Option<PathBuf>,
    /// Print the results as JSON.
    #[arg(long)]
    json: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(code) => code,
        Err(message) => {
            eprintln!("archie-search: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<ExitCode, String> {
    let query = match &args.query {
        Some(query) => query.clone(),
        None => read_stdin()?,
    };
    let manifest = manifest(&args)?;
    let mut registry = SearchRegistry::new();
    registry
        .register(manifest.clone())
        .map_err(|e| e.to_string())?;

    // The operator running this command is the grant.
    let authority = SearchAuthority::new([well_known::network_internet()].into_iter().collect());
    let proposal = SearchProposal::new(&manifest.id, query, args.privacy.into(), "cli", "cli")
        .with_options(SearchOptions {
            depth: args.depth.into(),
            topic: args.topic.into(),
            max_results: args.max_results,
            time_range: args.time_range.map(Into::into),
            include_domains: args.include_domains.clone(),
            exclude_domains: args.exclude_domains.clone(),
        });

    let search = match SearchGate::authorize(&registry, &authority, &proposal) {
        SearchAdmission::Allowed(search) => search,
        SearchAdmission::NeedsApproval { reason, .. } => {
            eprintln!("needs approval: {reason}");
            return Ok(ExitCode::from(3));
        }
        SearchAdmission::Refused(refusal) => {
            eprintln!("refused ({}): {refusal}", refusal.code());
            return Ok(ExitCode::from(2));
        }
    };

    let credential = broker_from_environment(search.credential_ref())?;
    let client = TavilyClient::connect(&manifest).map_err(|e| e.to_string())?;
    let results = client
        .search(&search, credential)
        .await
        .map_err(|e| format!("{e} ({})", e.code()))?;

    let receipt = search.receipt();
    eprintln!(
        "receipt: provider={} grade={} query_chars={} depth={} credits={} results={}",
        receipt.provider_id,
        receipt.grade,
        receipt.query_chars,
        receipt.depth.as_str(),
        receipt.credits,
        results.results.len()
    );
    if args.json {
        let text = serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?;
        println!("{text}");
    } else {
        print_results(&results);
    }
    Ok(ExitCode::SUCCESS)
}

fn manifest(args: &Args) -> Result<SearchProviderManifest, String> {
    let mut manifest = match &args.manifest {
        Some(path) => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            SearchRegistry::parse_manifest(&text).map_err(|e| e.to_string())?
        }
        None => SearchProviderManifest::tavily(
            VaultRef::parse(DEFAULT_CREDENTIAL).map_err(|e| e.to_string())?,
        ),
    };
    if args.zero_retention_verified {
        manifest.retention = DataRetention::ZeroVerified;
        manifest.max_privacy_grade = manifest.max_privacy_grade.max(PrivacyGrade::Internal);
    }
    if args.allow_advanced {
        manifest.allow_advanced_depth = true;
    }
    Ok(manifest)
}

/// Seal the key into a vault that lives only in this process, and broker it back out.
fn broker_from_environment(reference: &VaultRef) -> Result<SealedCredential, String> {
    let key = Zeroizing::new(
        std::env::var(KEY_ENV).map_err(|_| format!("set {KEY_ENV} to a Tavily API key"))?,
    );
    if key.trim().is_empty() {
        return Err(format!("{KEY_ENV} is empty"));
    }

    let mut seed = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| "the system random source failed".to_string())?;
    let vault_key = VaultKey::from_bytes(seed);
    seed.zeroize();

    let mut vault = Vault::open(&vault_key).map_err(|e| e.to_string())?;
    let now = now_ms();
    vault
        .store(
            ACTOR,
            reference.clone(),
            EntryClass::Credential,
            PrivacyGrade::Secret,
            key.trim(),
            now,
            None,
        )
        .map_err(|e| e.to_string())?;
    vault
        .broker_credential(ACTOR, reference, now)
        .map_err(|e| e.to_string())
}

fn read_stdin() -> Result<String, String> {
    if io::stdin().is_terminal() {
        return Err("give a query as an argument or on stdin".into());
    }
    let mut query = String::new();
    io::stdin()
        .read_to_string(&mut query)
        .map_err(|e| e.to_string())?;
    Ok(query)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(1)
}

fn print_results(results: &SearchResults) {
    println!(
        "{} result(s) from {} ({} discarded, {} truncated)",
        results.results.len(),
        results.provider_id,
        results.discarded,
        results.truncated
    );
    for (index, result) in results.results.iter().enumerate() {
        println!();
        println!("{}. {}", index + 1, result.title);
        println!("   {}", result.url);
        match &result.published {
            Some(published) => println!(
                "   {} · score {}/1000 · {}",
                published, result.score_permille, result.reference
            ),
            None => println!(
                "   score {}/1000 · {}",
                result.score_permille, result.reference
            ),
        }
        let preview: String = result
            .snippet
            .chars()
            .take(SNIPPET_PREVIEW_CHARS)
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect();
        if !preview.is_empty() {
            println!("   {preview}");
        }
    }
}
