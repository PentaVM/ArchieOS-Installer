//! The search gate: what reaches a provider, and what never does.

use archie_search::{
    SearchAdmission, SearchAuthority, SearchDepth, SearchGate, SearchOptions, SearchProposal,
    SearchProviderManifest, SearchRefusal, SearchRegistry, MAX_DOMAIN_FILTERS, MAX_QUERY_CHARS,
};
use archie_types::capability::well_known;
use archie_types::{Capability, CapabilitySet, DataRetention, PrivacyGrade};
use archie_vault::VaultRef;

const QUERY: &str = "ArchieOS capability-based scheduling";

fn tavily() -> SearchProviderManifest {
    let mut manifest =
        SearchProviderManifest::tavily(VaultRef::parse("vault:tavily/main").unwrap());
    manifest.max_results = 10;
    manifest
}

fn registry(manifest: SearchProviderManifest) -> SearchRegistry {
    let mut registry = SearchRegistry::new();
    registry.register(manifest).unwrap();
    registry
}

fn internet() -> SearchAuthority {
    SearchAuthority::new([well_known::network_internet()].into_iter().collect())
}

fn proposal(query: &str, grade: PrivacyGrade) -> SearchProposal {
    SearchProposal::new("tavily", query, grade, "task-1", "trace-1")
}

fn refusal(admission: SearchAdmission) -> SearchRefusal {
    admission
        .refusal()
        .cloned()
        .unwrap_or_else(|| panic!("expected a refusal, got {admission:?}"))
}

#[test]
fn a_public_query_is_admitted_with_the_manifest_key_and_budget() {
    let manifest = tavily();
    let admission = SearchGate::authorize(
        &registry(manifest.clone()),
        &internet(),
        &proposal(QUERY, PrivacyGrade::Public),
    );
    let search = admission
        .allowed()
        .expect("a public query to a public provider");

    assert_eq!(search.provider_id(), "tavily");
    assert_eq!(search.query(), QUERY);
    assert_eq!(search.credential_ref(), &manifest.secret_handle);
    assert_eq!(search.max_response_bytes(), manifest.max_response_bytes);
    assert_eq!(search.timeout_ms(), manifest.timeout_ms);
    assert_eq!(search.endpoint().as_str(), "https://api.tavily.com/");
}

#[test]
fn the_query_is_trimmed_but_otherwise_sent_as_written() {
    let admission = SearchGate::authorize(
        &registry(tavily()),
        &internet(),
        &proposal(&format!("   {QUERY}  "), PrivacyGrade::Public),
    );
    assert_eq!(admission.allowed().unwrap().query(), QUERY);
}

#[test]
fn an_unknown_or_disabled_provider_admits_nothing() {
    let admission = SearchGate::authorize(
        &registry(tavily()),
        &internet(),
        &SearchProposal::new("bing", QUERY, PrivacyGrade::Public, "t", "t"),
    );
    assert_eq!(
        refusal(admission),
        SearchRefusal::UnknownProvider("bing".into())
    );

    let mut disabled = tavily();
    disabled.enabled = false;
    let admission = SearchGate::authorize(
        &registry(disabled),
        &internet(),
        &proposal(QUERY, PrivacyGrade::Public),
    );
    assert_eq!(
        refusal(admission),
        SearchRefusal::ProviderDisabled("tavily".into())
    );
}

#[test]
fn searching_needs_the_internet_granted_whatever_the_manifest_says() {
    let admission = SearchGate::authorize(
        &registry(tavily()),
        &SearchAuthority::new(CapabilitySet::new()),
        &proposal(QUERY, PrivacyGrade::Public),
    );
    assert_eq!(
        refusal(admission),
        SearchRefusal::CapabilityNotGranted(well_known::network_internet())
    );

    // A neighbouring grant is not the internet.
    let lan = SearchAuthority::new([Capability::known("network.lan")].into_iter().collect());
    let admission = SearchGate::authorize(
        &registry(tavily()),
        &lan,
        &proposal(QUERY, PrivacyGrade::Public),
    );
    assert!(matches!(
        refusal(admission),
        SearchRefusal::CapabilityNotGranted(_)
    ));
}

#[test]
fn a_secret_query_is_refused_even_by_a_provider_on_this_machine() {
    let mut local = tavily();
    local.endpoint = "http://127.0.0.1:9000".into();
    local.max_privacy_grade = PrivacyGrade::Restricted;
    let admission = SearchGate::authorize(
        &registry(local),
        &internet(),
        &proposal(QUERY, PrivacyGrade::Secret),
    );
    assert_eq!(refusal(admission), SearchRefusal::SecretQuery);
}

#[test]
fn an_internal_query_needs_a_provider_whose_retention_was_verified() {
    let admission = SearchGate::authorize(
        &registry(tavily()),
        &internet(),
        &proposal(QUERY, PrivacyGrade::Internal),
    );
    assert_eq!(
        refusal(admission),
        SearchRefusal::PrivacyExceeded {
            provider_id: "tavily".into(),
            grade: PrivacyGrade::Internal
        }
    );

    let mut verified = tavily();
    verified.retention = DataRetention::ZeroVerified;
    verified.max_privacy_grade = PrivacyGrade::Internal;
    let admission = SearchGate::authorize(
        &registry(verified),
        &internet(),
        &proposal(QUERY, PrivacyGrade::Internal),
    );
    assert!(admission.allowed().is_some());

    // Verified retention does not make the cloud a place for sensitive material.
    let mut verified = tavily();
    verified.retention = DataRetention::ZeroVerified;
    verified.max_privacy_grade = PrivacyGrade::Internal;
    let admission = SearchGate::authorize(
        &registry(verified),
        &internet(),
        &proposal(QUERY, PrivacyGrade::Sensitive),
    );
    assert!(matches!(
        refusal(admission),
        SearchRefusal::PrivacyExceeded { .. }
    ));
}

#[test]
fn a_refusal_never_repeats_the_query() {
    let private = "my landlord's name and my bank";
    for (query, grade) in [
        (private.to_string(), PrivacyGrade::Internal),
        (format!("{private}\n{private}"), PrivacyGrade::Public),
        (private.repeat(MAX_QUERY_CHARS), PrivacyGrade::Public),
    ] {
        let admission =
            SearchGate::authorize(&registry(tavily()), &internet(), &proposal(&query, grade));
        let refusal = refusal(admission);
        assert!(!refusal.to_string().contains("landlord"), "{refusal}");
        assert!(!format!("{refusal:?}").contains("landlord"), "{refusal:?}");
        assert!(!refusal.code().contains(' '));
    }
}

#[test]
fn a_query_must_be_non_empty_bounded_plain_text() {
    let gate = |query: &str| {
        refusal(SearchGate::authorize(
            &registry(tavily()),
            &internet(),
            &proposal(query, PrivacyGrade::Public),
        ))
    };
    assert_eq!(gate("   "), SearchRefusal::EmptyQuery);
    assert_eq!(
        gate(&"ñ".repeat(MAX_QUERY_CHARS + 1)),
        SearchRefusal::QueryTooLong {
            chars: MAX_QUERY_CHARS + 1
        }
    );
    assert_eq!(
        gate("first line\nsecond line"),
        SearchRefusal::QueryNotPlainText
    );
    assert_eq!(gate("escape\u{1b}[31m"), SearchRefusal::QueryNotPlainText);

    // Exactly at the limit, counted in characters rather than bytes, is fine.
    let admission = SearchGate::authorize(
        &registry(tavily()),
        &internet(),
        &proposal(&"ñ".repeat(MAX_QUERY_CHARS), PrivacyGrade::Public),
    );
    assert!(admission.allowed().is_some());
}

#[test]
fn the_result_count_stays_within_the_manifest() {
    let with = |max_results: u8| {
        SearchGate::authorize(
            &registry(tavily()),
            &internet(),
            &proposal(QUERY, PrivacyGrade::Public).with_options(SearchOptions {
                max_results,
                ..SearchOptions::default()
            }),
        )
    };
    assert_eq!(
        refusal(with(0)),
        SearchRefusal::ResultCount {
            requested: 0,
            ceiling: 10
        }
    );
    assert_eq!(
        refusal(with(11)),
        SearchRefusal::ResultCount {
            requested: 11,
            ceiling: 10
        }
    );
    assert!(with(10).allowed().is_some());
}

#[test]
fn advanced_depth_costs_double_and_needs_the_operator_to_allow_it() {
    let advanced = |manifest: SearchProviderManifest| {
        SearchGate::authorize(
            &registry(manifest),
            &internet(),
            &proposal(QUERY, PrivacyGrade::Public).with_options(SearchOptions {
                depth: SearchDepth::Advanced,
                ..SearchOptions::default()
            }),
        )
    };
    assert_eq!(
        refusal(advanced(tavily())),
        SearchRefusal::DepthNotAllowed("tavily".into())
    );
    let mut allowed = tavily();
    allowed.allow_advanced_depth = true;
    let admission = advanced(allowed);
    assert_eq!(admission.allowed().unwrap().receipt().credits, 2);
}

#[test]
fn domain_filters_are_hostnames_bounded_and_consistent() {
    let with = |include: Vec<&str>, exclude: Vec<&str>| {
        SearchGate::authorize(
            &registry(tavily()),
            &internet(),
            &proposal(QUERY, PrivacyGrade::Public).with_options(SearchOptions {
                include_domains: include.into_iter().map(String::from).collect(),
                exclude_domains: exclude.into_iter().map(String::from).collect(),
                ..SearchOptions::default()
            }),
        )
    };

    let admission = with(vec![" ArXiv.org "], vec!["pinterest.com"]);
    let search = admission.allowed().expect("valid filters");
    assert_eq!(search.options().include_domains, vec!["arxiv.org"]);
    assert_eq!(search.options().exclude_domains, vec!["pinterest.com"]);

    assert_eq!(
        refusal(with(vec!["https://arxiv.org/abs"], vec![])),
        SearchRefusal::InvalidDomain
    );
    assert_eq!(
        refusal(with(vec!["arxiv.org"], vec!["ARXIV.org"])),
        SearchRefusal::DomainInBothLists
    );
    let many: Vec<String> = (0..=MAX_DOMAIN_FILTERS)
        .map(|i| format!("site{i}.example"))
        .collect();
    assert_eq!(
        refusal(with(many.iter().map(String::as_str).collect(), vec![])),
        SearchRefusal::TooManyDomainFilters {
            count: MAX_DOMAIN_FILTERS + 1
        }
    );
}

#[test]
fn an_operator_can_require_approval_for_every_query_to_a_provider() {
    let authority = internet().requiring_approval("tavily");
    match SearchGate::authorize(
        &registry(tavily()),
        &authority,
        &proposal(QUERY, PrivacyGrade::Public),
    ) {
        SearchAdmission::NeedsApproval { search, .. } => assert_eq!(search.query(), QUERY),
        other => panic!("expected approval to be required, got {other:?}"),
    }
}

#[test]
fn the_receipt_measures_the_query_and_does_not_contain_it() {
    let admission = SearchGate::authorize(
        &registry(tavily()),
        &internet(),
        &proposal(QUERY, PrivacyGrade::Public).with_options(SearchOptions {
            include_domains: vec!["arxiv.org".into()],
            ..SearchOptions::default()
        }),
    );
    let receipt = admission.allowed().unwrap().receipt();
    assert_eq!(receipt.query_chars, QUERY.chars().count());
    assert_eq!(receipt.domain_filters, 1);
    assert_eq!(receipt.credits, 1);

    let written = serde_json::to_string(&receipt).unwrap();
    assert!(!written.contains("scheduling"), "{written}");
    assert!(!written.contains("arxiv"), "{written}");
    assert!(
        written.contains("vault:tavily/main"),
        "the key's name, never the key"
    );
}

#[test]
fn debug_output_measures_the_query_instead_of_printing_it() {
    let proposal =
        proposal("my landlord's name", PrivacyGrade::Public).with_options(SearchOptions {
            include_domains: vec!["private-clinic.example".into()],
            ..SearchOptions::default()
        });
    let admission = SearchGate::authorize(&registry(tavily()), &internet(), &proposal);
    let search = admission.allowed().expect("admitted");
    for rendered in [
        format!("{proposal:?}"),
        format!("{search:?}"),
        format!("{admission:?}"),
    ] {
        assert!(!rendered.contains("landlord"), "{rendered}");
        assert!(!rendered.contains("clinic"), "{rendered}");
        assert!(rendered.contains("query_chars: 18"), "{rendered}");
    }
}

#[test]
fn every_shipped_search_manifest_parses_and_validates() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate sits three levels below the repository root")
        .join("providers/search");
    let registry = SearchRegistry::load_from_dir(&dir).expect("shipped manifests must load");
    let tavily = registry
        .get("tavily")
        .expect("Tavily ships as a search provider");
    assert!(tavily.enabled);
    // Shipped conservatively: nobody has verified this account's retention.
    assert_eq!(tavily.max_privacy_grade, PrivacyGrade::Public);
    assert_eq!(tavily.retention, DataRetention::Unknown);
    assert!(!tavily.allow_advanced_depth);
}
