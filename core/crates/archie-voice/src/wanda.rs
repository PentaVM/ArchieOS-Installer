//! Private Wanda proposals return only to a validated local cognitive session.
use archie_model_runtime::{ProviderEndpoint, ProviderFamily, ToolDefinition};
use archie_types::{Locality, ModelDescriptor, PrivacyGrade, ProviderKind};

#[derive(Clone, Copy, Debug, Default)]
pub struct WandaVoiceAccess {
    allowed: bool,
}

impl WandaVoiceAccess {
    pub(crate) fn from_selected(
        endpoint: &ProviderEndpoint,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
    ) -> Self {
        let provider_matches = matches!(
            (endpoint.family(), &descriptor.provider),
            (ProviderFamily::NvidiaNim, ProviderKind::NvidiaNim)
                | (
                    ProviderFamily::OpenAiCompatible,
                    ProviderKind::OpenAiCompatible
                )
        );
        let loopback = endpoint
            .base_url()
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
        Self {
            allowed: provider_matches
                && loopback
                && endpoint.locality() == Locality::Local
                && descriptor.locality == Locality::Local
                && descriptor.locality_is_consistent()
                && descriptor.permits_privacy(PrivacyGrade::Restricted)
                && privacy == PrivacyGrade::Restricted,
        }
    }
    pub fn permits_private_context(self) -> bool {
        self.allowed
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct WandaVoiceRequest {
    pub domain: String,
    pub question: String,
    pub deep: bool,
}

pub struct WandaVoice {
    access: WandaVoiceAccess,
    domain: String,
}
pub fn bounded_answer(value: &str) -> String {
    format!("Propuesta de Wanda para revisar: {value}")
        .chars()
        .take(1200)
        .collect()
}
impl WandaVoice {
    pub fn new(access: WandaVoiceAccess, domain: String) -> Self {
        Self { access, domain }
    }
    pub fn from_env(access: WandaVoiceAccess) -> Self {
        Self::new(
            access,
            std::env::var("ARCHIE_WANDA_DOMAIN")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "personal/owner".into()),
        )
    }
    /// Gate before reading credentials or fetching Wanda details. The model cannot
    /// supply locality, scope, or a domain override.
    pub fn plan(&self, raw: &str) -> Result<WandaVoiceRequest, &'static str> {
        if !self.access.permits_private_context() {
            return Err("private_voice_required");
        }
        if raw.len() > 16 * 1024 {
            return Err("invalid_wanda_arguments");
        }
        let value: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| "invalid_wanda_arguments")?;
        let object = value.as_object().ok_or("invalid_wanda_arguments")?;
        if object.keys().any(|key| key != "question" && key != "deep") {
            return Err("invalid_wanda_arguments");
        }
        let question = object
            .get("question")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .ok_or("invalid_wanda_arguments")?;
        if question.is_empty()
            || question.chars().count() > 2000
            || question
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        {
            return Err("invalid_wanda_arguments");
        }
        let deep = match object.get("deep") {
            None => false,
            Some(value) => value.as_bool().ok_or("invalid_wanda_arguments")?,
        };
        Ok(WandaVoiceRequest {
            domain: self.domain.clone(),
            question: question.into(),
            deep,
        })
    }
    #[cfg(feature = "wanda")]
    pub async fn handle_tool(&self, raw: &str) -> String {
        let request=match self.plan(raw) {
            Ok(request)=>request,
            Err("private_voice_required")=>return "No consulté contexto privado: Wanda necesita una sesión de voz con inferencia local validada.".into(),
            Err(_)=>return "No consulté Wanda: los argumentos de la consulta no son válidos.".into(),
        };
        // A denied session reaches neither token loading nor HTTP. No client/service
        // error is echoed: it may contain private paths or operator configuration.
        let client = match archie_wanda::client::WandaClient::from_env() {
            Ok(Some(client)) => client,
            Ok(None) => return "Wanda todavía no está configurada en este equipo.".into(),
            Err(_) => return "No pude conectar con Wanda. Revisa su configuración local.".into(),
        };
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.query(&request.domain, &request.question, request.deep),
        )
        .await
        {
            // The crisis rule's fixed message is said as it is, never framed as a proposal.
            Ok(Ok(response)) if response.state == "crisis" => {
                response.speech_text().chars().take(1200).collect()
            }
            Ok(Ok(response)) => bounded_answer(&response.speech_text()),
            Ok(Err(_)) => {
                "No pude consultar Wanda. Revisa el servicio local y sus permisos.".into()
            }
            Err(_) => "Wanda no respondió a tiempo. Puedes volver a consultar.".into(),
        }
    }
}

pub fn tool() -> ToolDefinition {
    ToolDefinition {
    name: "wanda_context".into(),
    description:"Consulta a Wanda para proponer un regalo, revisar compromisos o preparar una respuesta según las personas, preferencias y evidencias del contexto configurado. Sólo en una sesión de voz local; las propuestas no confirman relaciones ni ejecutan acciones.".into(),
    parameters:serde_json::json!({"type":"object","additionalProperties":false,
        "properties":{"question":{"type":"string","maxLength":2000,"description":"Qué pidió la persona, sin añadir hechos."},
            "deep":{"type":"boolean","description":"Pedir razonamiento local más profundo."}},"required":["question"]})
}
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::{
        Availability, Capability, DataRetention, Locality, ModelId, ProviderKind, TokenCost,
    };
    use std::collections::BTreeMap;
    fn descriptor(
        provider: ProviderKind,
        locality: Locality,
        grade: PrivacyGrade,
    ) -> ModelDescriptor {
        ModelDescriptor {
            id: ModelId::parse("test.voice").unwrap(),
            display_name: "test".into(),
            provider,
            locality,
            node: None,
            context_window: 32000,
            max_output_tokens: 400,
            capabilities: vec![Capability::known("compute.inference")],
            max_privacy_grade: grade,
            retention: DataRetention::ZeroVerified,
            availability: Availability::Ready,
            cost: TokenCost::FREE,
            latency_ema_ms: None,
            quality_tier: 50,
            required_accelerator_bytes: None,
            labels: BTreeMap::new(),
        }
    }
    fn local() -> WandaVoiceAccess {
        WandaVoiceAccess::from_selected(
            &ProviderEndpoint::nvidia_nim("http://127.0.0.1:8000/v1", Locality::Local).unwrap(),
            &descriptor(
                ProviderKind::NvidiaNim,
                Locality::Local,
                PrivacyGrade::Restricted,
            ),
            PrivacyGrade::Restricted,
        )
    }
    #[test]
    fn only_selected_loopback_source_with_restricted_grade_can_receive_details() {
        assert!(local().permits_private_context());
        let cloud = ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1").unwrap();
        assert!(!WandaVoiceAccess::from_selected(
            &cloud,
            &descriptor(
                ProviderKind::NebiusTokenFactory,
                Locality::Cloud,
                PrivacyGrade::Public
            ),
            PrivacyGrade::Public
        )
        .permits_private_context());
        let remote =
            ProviderEndpoint::nvidia_nim("https://inference.example/v1", Locality::Local).unwrap();
        assert!(!WandaVoiceAccess::from_selected(
            &remote,
            &descriptor(
                ProviderKind::NvidiaNim,
                Locality::Local,
                PrivacyGrade::Restricted
            ),
            PrivacyGrade::Restricted
        )
        .permits_private_context());
        let endpoint =
            ProviderEndpoint::nvidia_nim("http://localhost:8000/v1", Locality::Local).unwrap();
        assert!(!WandaVoiceAccess::from_selected(
            &endpoint,
            &descriptor(
                ProviderKind::NebiusTokenFactory,
                Locality::Local,
                PrivacyGrade::Restricted
            ),
            PrivacyGrade::Restricted
        )
        .permits_private_context());
        assert!(!WandaVoiceAccess::from_selected(
            &endpoint,
            &descriptor(
                ProviderKind::NvidiaNim,
                Locality::Local,
                PrivacyGrade::Internal
            ),
            PrivacyGrade::Restricted
        )
        .permits_private_context());
        assert!(!WandaVoiceAccess::from_selected(
            &endpoint,
            &descriptor(
                ProviderKind::NvidiaNim,
                Locality::Local,
                PrivacyGrade::Restricted
            ),
            PrivacyGrade::Internal
        )
        .permits_private_context());
    }
    #[test]
    fn denied_voice_never_parses_or_echoes_private_arguments() {
        let voice = WandaVoice::new(WandaVoiceAccess::default(), "personal/owner".into());
        assert_eq!(
            voice.plan(r#"{"question":"private family detail", "local":true}"#),
            Err("private_voice_required")
        );
        assert_eq!(
            voice.plan("invalid JSON with private family detail"),
            Err("private_voice_required")
        );
    }
    #[test]
    fn trusted_domain_is_fixed_and_tool_arguments_are_strict_and_bounded() {
        let voice = WandaVoice::new(local(), "work/studio/project".into());
        assert_eq!(
            voice
                .plan(r#"{"question":"  ¿Qué debería considerar?  ","deep":true}"#)
                .unwrap(),
            WandaVoiceRequest {
                domain: "work/studio/project".into(),
                question: "¿Qué debería considerar?".into(),
                deep: true
            }
        );
        for raw in [
            r#"{"question":"algo","domain":"personal/owner"}"#,
            r#"{"question":"algo","local":true}"#,
            r#"{"question":""}"#,
            r#"{"question":null}"#,
            r#"{"question":"algo","deep":"yes"}"#,
            "[]",
        ] {
            assert_eq!(voice.plan(raw), Err("invalid_wanda_arguments"));
        }
        assert!(voice
            .plan(&serde_json::json!({"question":"a".repeat(2000)}).to_string())
            .is_ok());
        assert_eq!(
            voice.plan(&serde_json::json!({"question":"a".repeat(2001)}).to_string()),
            Err("invalid_wanda_arguments")
        );
    }
    #[test]
    fn tool_schema_exposes_no_domain_or_privacy_override() {
        let tool = tool();
        assert_eq!(tool.parameters["required"], serde_json::json!(["question"]));
        assert_eq!(tool.parameters["additionalProperties"], false);
        assert!(tool.parameters["properties"].get("domain").is_none());
        assert!(tool.parameters["properties"].get("local").is_none());
    }
    #[test]
    fn source_proof_is_bound_to_its_constructor_and_legacy_client_defaults_to_denied() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let endpoint =
            ProviderEndpoint::nvidia_nim("http://127.0.0.1:8000/v1", Locality::Local).unwrap();
        let model = descriptor(
            ProviderKind::NvidiaNim,
            Locality::Local,
            PrivacyGrade::Restricted,
        );
        let legacy = crate::NebiusSource::new(
            archie_model_runtime::InferenceClient::new(endpoint.clone()).unwrap(),
            model.clone(),
            PrivacyGrade::Restricted,
            runtime.handle().clone(),
        );
        assert!(!legacy.wanda_access().permits_private_context());
        let chosen = crate::NebiusSource::from_endpoint(
            endpoint,
            model,
            PrivacyGrade::Restricted,
            runtime.handle().clone(),
        )
        .unwrap();
        assert!(chosen.wanda_access().permits_private_context());
    }
    #[test]
    fn tool_results_remain_advisory_and_bounded_even_for_long_unavailable_reasons() {
        let answer = bounded_answer(&"é".repeat(2000));
        assert!(answer.chars().count() <= 1200);
        assert!(answer.starts_with("Propuesta de Wanda para revisar: "));
    }
    #[cfg(feature = "wanda")]
    #[tokio::test]
    async fn cloud_dispatch_refuses_before_credentials_or_any_detail_request() {
        let voice = WandaVoice::new(
            WandaVoiceAccess::default(),
            "personal/private-domain".into(),
        );
        let reply = voice
            .handle_tool(r#"{"question":"secret preference", "domain":"work/private"}"#)
            .await;
        assert_eq!(reply,"No consulté contexto privado: Wanda necesita una sesión de voz con inferencia local validada.");
        assert!(!reply.contains("secret preference"));
        assert!(!reply.contains("private-domain"));
    }
}
