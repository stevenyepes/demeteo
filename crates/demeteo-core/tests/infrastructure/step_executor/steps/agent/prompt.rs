// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/agent/mod.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn rc(feedback: &str) -> RetryContext {
    RetryContext {
        feedback: feedback.into(),
        iteration: 1,
        max: 1,
        failing_tests: Vec::new(),
        implicated_files: Vec::new(),
        failing_step_id: String::new(),
    }
}

// ── format_retry_feedback_section ────────────────────────────────────

#[test]
fn format_returns_empty_when_no_retry_ctx() {
    assert_eq!(format_retry_feedback_section(None), "");
}

#[test]
fn format_returns_empty_when_feedback_is_whitespace() {
    assert_eq!(format_retry_feedback_section(Some(&rc("   \n\t"))), "");
}

#[test]
fn format_returns_section_text_when_feedback_present() {
    let s = format_retry_feedback_section(Some(&rc("use cargo before mise")));
    assert!(s.contains("## Previous Attempt Feedback"));
    assert!(s.contains("use cargo before mise"));
}

// ── template_uses_retry_section ──────────────────────────────────────

#[test]
fn detects_placeholder_presence() {
    assert!(template_uses_retry_section(
        "hello {{retry_feedback_section}} world"
    ));
    assert!(!template_uses_retry_section(
        "hello {{retry_feedback}} world"
    ));
    assert!(!template_uses_retry_section(""));
}

// ── append_retry_feedback_section (safety-net fallback) ──────────────

#[test]
fn first_attempt_leaves_prompt_unchanged() {
    let prompt = "do the thing".to_string();
    let result = append_retry_feedback_section(prompt.clone(), None);
    assert_eq!(result, prompt);
}

#[test]
fn retry_with_empty_feedback_leaves_prompt_unchanged() {
    let prompt = "do the thing".to_string();
    let result = append_retry_feedback_section(prompt.clone(), Some(&rc("   ")));
    assert_eq!(result, prompt, "whitespace-only feedback must not append");
}

#[test]
fn retry_with_feedback_appends_section() {
    let prompt = "do the thing".to_string();
    let result = append_retry_feedback_section(prompt, Some(&rc("use cargo before mise")));
    assert!(result.starts_with("do the thing"));
    assert!(result.contains("## Previous Attempt Feedback"));
    assert!(result.contains("use cargo before mise"));
}

#[test]
fn retry_section_appears_after_template_content() {
    let result = append_retry_feedback_section(
        "research the codebase".into(),
        Some(&rc("also check the docs/ folder")),
    );
    let template_end =
        result.find("research the codebase").unwrap() + "research the codebase".len();
    let section_start = result.find("## Previous Attempt Feedback").unwrap();
    assert!(
        section_start > template_end,
        "feedback section must come after the rendered template"
    );
}

// ── combined: placement-by-placeholder behavior ─────────────────────

#[test]
fn template_with_placeholder_renders_section_inline() {
    // Template that opts into placement-by-placeholder. The
    // caller would NOT call append_retry_feedback_section in
    // this branch (template_uses_retry_section returns true).
    let template = "intro {{retry_feedback_section}} outro";
    let section = format_retry_feedback_section(Some(&rc("use cargo before mise")));
    assert!(section.contains("use cargo before mise"));

    let rendered = template.replace("{{retry_feedback_section}}", &section);
    assert!(rendered.contains("intro "));
    assert!(rendered.contains(" outro"));
    assert!(rendered.contains("## Previous Attempt Feedback"));
    // The placeholder is gone — fully substituted.
    assert!(!rendered.contains("{{retry_feedback_section}}"));
}

#[test]
fn template_without_placeholder_gets_safety_net_append() {
    // Template that doesn't reference the placeholder — system
    // auto-appends so feedback still reaches the agent.
    let rendered = "intro".to_string();
    let after_safety_net =
        append_retry_feedback_section(rendered, Some(&rc("use cargo before mise")));
    assert!(after_safety_net.contains("intro"));
    assert!(after_safety_net.contains("## Previous Attempt Feedback"));
    assert!(after_safety_net.contains("use cargo before mise"));
}

#[test]
fn placeholder_empty_when_no_retry_no_visual_artifact() {
    // A template that references the placeholder even on first
    // attempts must render cleanly — no leftover "---" or empty
    // section header.
    let template = "intro {{retry_feedback_section}} outro";
    let section = format_retry_feedback_section(None);
    assert_eq!(section, "");
    let rendered = template.replace("{{retry_feedback_section}}", &section);
    assert_eq!(
        rendered, "intro  outro",
        "empty section must collapse cleanly"
    );
}

// ── needs_harness_briefing ──────────────────────────────────────────
//
// The predicate is the *only* thing standing between a template that
// never mentions the baseline and two DB reads on every attempt. It is
// asserted here rather than against the driver because reaching the
// branch itself would mean standing up an `ExecutionDriver` and its
// eighteen ports to observe two of them.

#[test]
fn a_template_without_the_token_asks_for_no_briefing() {
    assert!(!needs_harness_briefing(""));
    assert!(!needs_harness_briefing(
        "Implement {{feature_description}} and run {{test_command}}."
    ));
    assert!(
        !needs_harness_briefing("{{harness_delta}} {{harness}}"),
        "a near-miss token must not pay for the briefing"
    );
}

#[test]
fn a_template_naming_the_token_asks_for_the_briefing() {
    assert!(needs_harness_briefing("{{harness_baseline}}"));
    assert!(needs_harness_briefing(
        "## Gates\n{{harness_baseline}}\n\n## Task\n…"
    ));
}

// ── needs_gate_decision_log ─────────────────────────────────────────
//
// Same opt-in shape as the briefing above, plus one thing the briefing
// has no equivalent of: the singular `{{gate_decision}}` sits right next
// to it, and the two must not be confusable in either direction.

#[test]
fn a_template_without_the_token_asks_for_no_gate_log() {
    assert!(!needs_gate_decision_log(""));
    assert!(!needs_gate_decision_log(
        "Implement {{feature_description}}."
    ));
}

#[test]
fn a_template_naming_the_token_asks_for_the_gate_log() {
    assert!(needs_gate_decision_log("{{gate_decision_log}}"));
    assert!(needs_gate_decision_log(
        "## Decisions\n{{gate_decision_log}}\n\n## Task\n…"
    ));
}

/// The singular latest-decision binding must not buy the history, or every
/// step that only wanted "what did the last gate say" pays for a query it
/// never reads.
#[test]
fn the_singular_gate_decision_token_does_not_ask_for_the_log() {
    assert!(!needs_gate_decision_log(
        "{{gate_decision}} {{gate_feedback}}"
    ));
}

/// The renderer is `String::replace` per key in insertion order, and
/// `gate_decision` is bound before `gate_decision_log`. The suffix is what
/// keeps the shorter token from eating the longer one — assert it rather
/// than trusting that the two spellings happen not to overlap.
#[test]
fn binding_the_singular_token_first_leaves_the_log_token_intact() {
    let rendered = crate::domain::prompt_context::PromptContext::new()
        .set("gate_decision", "approve")
        .set("gate_decision_log", "THE-LOG")
        .render("decision={{gate_decision}} log={{gate_decision_log}}");
    assert_eq!(rendered, "decision=approve log=THE-LOG");
}

// ── attachment_context_dir ──────────────────────────────────────────

fn attached(name: &str) -> AttachedFile {
    AttachedFile {
        id: format!("att-{name}"),
        name: name.into(),
        mime: "image/png".into(),
        sha256: "0".repeat(64),
        size: 1,
        source_filename: format!("{name}.png"),
    }
}

#[test]
fn no_attachments_means_no_context_dir() {
    assert_eq!(attachment_context_dir("/repo/wt", &[]), None);
}

#[test]
fn attachments_point_at_the_worktree_local_copies() {
    let dir = attachment_context_dir("/repo/wt", &[attached("shot")])
        .expect("a non-empty manifest must yield a directory");
    let expected = std::path::Path::new("/repo/wt")
        .join("_context")
        .join("attachments")
        .to_string_lossy()
        .to_string();
    assert_eq!(dir, expected, "built with Path::join, not concatenation");
    assert!(dir.starts_with("/repo/wt"));
}

// ── append_verdict_contract ─────────────────────────────────────────

fn verifier(key: &str) -> VerifierConfig {
    VerifierConfig {
        agent_kind: None,
        model: None,
        effort: None,
        instructions: "Judge the acceptance criteria.".into(),
        harness_names: Vec::new(),
        verdict_key: key.into(),
    }
}

fn green_harness() -> HarnessOutcome {
    HarnessOutcome::from_runs(vec![crate::domain::harness_outcome::HarnessRun {
        name: "unit".into(),
        cmd: "cargo test".into(),
        output: "ok".into(),
    }])
}

#[test]
fn a_step_with_no_verifier_is_asked_for_no_verdict() {
    let cfg = verifier("verdict");
    let outcome = green_harness();
    assert_eq!(
        append_verdict_contract("body".into(), None, Some(&outcome)),
        "body"
    );
    assert_eq!(
        append_verdict_contract("body".into(), Some(&cfg), None),
        "body"
    );
    assert_eq!(append_verdict_contract("body".into(), None, None), "body");
}

#[test]
fn a_verified_step_gets_the_harness_section_then_the_contract() {
    let cfg = verifier("verdict");
    let outcome = green_harness();
    let out = append_verdict_contract("body".into(), Some(&cfg), Some(&outcome));

    assert!(out.starts_with("body"));
    let section = out
        .find("cargo test")
        .expect("the harness section must render its own heading and commands");
    let heading = out
        .find("## Required Verdict")
        .expect("the verdict heading must be present");
    let instructions = out
        .find("Judge the acceptance criteria.")
        .expect("the verifier's own instructions must be present");
    assert!(
        section < heading && heading < instructions,
        "harness output, then the verdict heading, then the instructions"
    );
    assert!(
        instructions < out.find("\"pass\"").expect("the menu must be present"),
        "the contract menu comes last"
    );
}

#[test]
fn the_contract_uses_the_configured_verdict_key() {
    let cfg = verifier("acceptance");
    let outcome = green_harness();
    let out = append_verdict_contract("body".into(), Some(&cfg), Some(&outcome));
    assert!(out.contains("\"acceptance\""));
}

#[test]
fn all_three_verdicts_are_offered_s13() {
    let cfg = verifier("verdict");
    let outcome = green_harness();
    let out = append_verdict_contract("body".into(), Some(&cfg), Some(&outcome));
    for verdict in ["pass", "fail", "environment"] {
        assert!(
            out.contains(&format!("\"{verdict}\"")),
            "an agent that cannot see `{verdict}` in the menu cannot answer it (S13)"
        );
    }
}
