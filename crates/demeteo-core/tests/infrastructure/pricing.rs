use super::*;

#[test]
fn exact_match_known_model() {
    let t = HardcodedPricingTable::new();
    let p = t.price_for("claude-sonnet-4").unwrap();
    assert_eq!(p.input_per_million, 3.00);
    assert_eq!(p.output_per_million, 15.00);
}

#[test]
fn prefix_match_claude_aliased() {
    let t = HardcodedPricingTable::new();
    let p = t.price_for("claude-3-5-sonnet-20241022").unwrap();
    assert_eq!(p.input_per_million, 3.00);
}

#[test]
fn case_insensitive() {
    let t = HardcodedPricingTable::new();
    assert!(t.price_for("Claude-Sonnet-4").is_some());
}

#[test]
fn unknown_model_returns_none() {
    let t = HardcodedPricingTable::new();
    assert!(t.price_for("not-a-real-model-9000").is_none());
}

#[test]
fn empty_model_returns_none() {
    let t = HardcodedPricingTable::new();
    assert!(t.price_for("   ").is_none());
}

#[test]
fn free_model_costs_zero() {
    let t = HardcodedPricingTable::new();
    let p = t.price_for("ollama").unwrap();
    assert_eq!(p.cost_usd(1_000_000, 1_000_000), 0.0);
}

#[test]
fn cost_calculation_proportional() {
    let t = HardcodedPricingTable::new();
    let p = t.price_for("gpt-4o").unwrap();
    assert!((p.cost_usd(1_000_000, 1_000_000) - 12.50).abs() < 1e-9);
    assert!((p.cost_usd(1_000, 0) - 0.0025).abs() < 1e-9);
}

#[test]
fn known_models_is_nonempty_and_sorted() {
    let t = HardcodedPricingTable::new();
    let v = t.known_models();
    assert!(
        v.len() >= 5,
        "R1 requires 5-10 known models, got {}",
        v.len()
    );
    let mut sorted = v.clone();
    sorted.sort();
    assert_eq!(v, sorted, "known_models() should be sorted");
}

// ── context_window (token-optimization Tier 1) ─────────────────────────

#[test]
fn context_window_known_claude_models() {
    let t = HardcodedPricingTable::new();
    assert_eq!(t.context_window("claude-sonnet-4"), Some(200_000));
    assert_eq!(t.context_window("claude-opus-4"), Some(200_000));
    assert_eq!(t.context_window("claude-haiku-4"), Some(200_000));
}

#[test]
fn context_window_prefix_fallback_aliased() {
    let t = HardcodedPricingTable::new();
    // Aliased date-stamped id still resolves through the prefix match.
    assert_eq!(
        t.context_window("claude-3-5-sonnet-20241022"),
        Some(200_000)
    );
}

#[test]
fn context_window_gpt_family() {
    let t = HardcodedPricingTable::new();
    assert_eq!(t.context_window("gpt-4o"), Some(128_000));
    assert_eq!(t.context_window("o1"), Some(200_000));
}

#[test]
fn context_window_gemini_family() {
    let t = HardcodedPricingTable::new();
    assert_eq!(t.context_window("gemini-2.5-pro"), Some(200_000));
    assert_eq!(t.context_window("gemini-pro"), Some(100_000));
}

#[test]
fn context_window_local_models_is_none() {
    let t = HardcodedPricingTable::new();
    // Local / free models have no enforced window — the watchdog
    // treats `None` as "no budget data, skip check."
    assert_eq!(t.context_window("ollama"), None);
    assert_eq!(t.context_window("llama"), None);
    assert_eq!(t.context_window("local"), None);
}

#[test]
fn context_window_unknown_model_is_none() {
    let t = HardcodedPricingTable::new();
    assert_eq!(t.context_window("not-a-real-model-9000"), None);
    assert_eq!(t.context_window(""), None);
    assert_eq!(t.context_window("   "), None);
}

#[test]
fn context_window_case_insensitive() {
    let t = HardcodedPricingTable::new();
    assert_eq!(t.context_window("Claude-Sonnet-4"), Some(200_000));
}
