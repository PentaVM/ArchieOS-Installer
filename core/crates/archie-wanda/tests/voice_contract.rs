use archie_wanda::types::QueryResponse;

#[test]
fn failure_text_is_bounded_before_it_can_enter_the_voice_floor() {
    let response = QueryResponse {
        recommendations: vec![],
        model: None,
        state: "unavailable".into(),
        reason: Some("é".repeat(10_000)),
        quick: None,
        refined: None,
        receipt: None,
    };
    assert!(response.speech_text().chars().count() <= 1200);
}
