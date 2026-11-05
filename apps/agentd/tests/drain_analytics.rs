use agentd::drain::Drain;

#[test]
fn recommendation_requires_measured_baseline() {
    let d = Drain::new();
    assert!(d.recommend_off("mic", 8.0).is_none());
}

#[test]
fn measured_history_produces_cited_estimate() {
    let mut d = Drain::new();
    d.record("mic", 3.0);
    let msg = d.recommend_off("mic", 8.0).unwrap();
    assert!(msg.contains("mic"));
}
