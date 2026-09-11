use archie_model_runtime::{
    ChatMessage, ChatRequest, InferenceClient, MessageRole, ProviderEndpoint,
};
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost,
};
use clap::{Parser, ValueEnum};
use std::collections::BTreeMap;
use std::io::{self, Read};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderArg {
    NvidiaNim,
    Nebius,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LocalityArg {
    Local,
    Fabric,
    Cloud,
}

impl From<LocalityArg> for Locality {
    fn from(value: LocalityArg) -> Self {
        match value {
            LocalityArg::Local => Self::Local,
            LocalityArg::Fabric => Self::Fabric,
            LocalityArg::Cloud => Self::Cloud,
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
#[command(about = "Policy-gated smoke client for NVIDIA NIM and Nebius")]
struct Args {
    #[arg(long, value_enum)]
    provider: ProviderArg,
    #[arg(long)]
    base_url: String,
    #[arg(long, value_enum, default_value = "cloud")]
    locality: LocalityArg,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, value_enum, default_value = "internal")]
    privacy: PrivacyArg,
    /// Assert that zero data retention has been verified for this account and
    /// endpoint. Without it a cloud provider may only receive public data, because an
    /// unverified retention posture is indistinguishable from one that keeps the
    /// prompt.
    #[arg(long)]
    zero_retention_verified: bool,
    #[arg(long)]
    list_models: bool,
    #[arg(long)]
    health: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let locality: Locality = args.locality.into();
    let (endpoint, provider_kind) = match args.provider {
        ProviderArg::NvidiaNim => (
            ProviderEndpoint::nvidia_nim(&args.base_url, locality)?,
            ProviderKind::NvidiaNim,
        ),
        ProviderArg::Nebius => (
            ProviderEndpoint::nebius(&args.base_url)?,
            ProviderKind::NebiusTokenFactory,
        ),
    };
    let client = InferenceClient::new(endpoint)?;

    if args.health {
        println!(
            "{}",
            if client.ready().await? {
                "ready"
            } else {
                "not-ready"
            }
        );
        return Ok(());
    }
    if args.list_models {
        for model in client.models().await?.data {
            println!("{}", model.id);
        }
        return Ok(());
    }

    let model = args.model.ok_or("--model is required for inference")?;
    let mut prompt = String::new();
    io::stdin().read_to_string(&mut prompt)?;
    if prompt.trim().is_empty() {
        return Err("provide the prompt on stdin".into());
    }
    let privacy: PrivacyGrade = args.privacy.into();
    let descriptor = ModelDescriptor {
        id: ModelId::parse(&model)?,
        display_name: model.clone(),
        provider: provider_kind,
        locality,
        node: None,
        context_window: 131_072,
        max_output_tokens: 16_384,
        capabilities: vec![Capability::known("compute.inference")],
        max_privacy_grade: if locality == Locality::Cloud {
            PrivacyGrade::Internal
        } else {
            PrivacyGrade::Restricted
        },
        retention: if args.zero_retention_verified {
            DataRetention::ZeroVerified
        } else {
            DataRetention::Unknown
        },
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: None,
        quality_tier: 0,
        required_accelerator_bytes: None,
        labels: BTreeMap::new(),
    };
    let request = ChatRequest {
        model,
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: prompt,
        }],
        max_tokens: Some(1024),
        temperature: Some(0.2),
        stream: false,
    };
    let response = client.chat(&descriptor, privacy, &request).await?;
    print!("{}", response.choices[0].message.content);
    Ok(())
}
