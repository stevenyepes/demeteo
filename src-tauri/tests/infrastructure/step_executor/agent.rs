use crate::adapters::step_executor::driver::RetryContext;
use super::{
    format_retry_feedback_section, template_uses_retry_section, append_retry_feedback_section,
};

fn rc(feedback: &str) -> RetryContext {
    RetryContext {
        feedback: feedback.into(),
        iteration: 1,
        max: 1,
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
    let template = "intro {{retry_feedback_section}} outro";
    let section = format_retry_feedback_section(Some(&rc("use cargo before mise")));
    assert!(section.contains("use cargo before mise"));

    let rendered = template.replace("{{retry_feedback_section}}", &section);
    assert!(rendered.contains("intro "));
    assert!(rendered.contains(" outro"));
    assert!(rendered.contains("## Previous Attempt Feedback"));
    assert!(!rendered.contains("{{retry_feedback_section}}"));
}

#[test]
fn template_without_placeholder_gets_safety_net_append() {
    let rendered = "intro".to_string();
    let after_safety_net =
        append_retry_feedback_section(rendered, Some(&rc("use cargo before mise")));
    assert!(after_safety_net.contains("intro"));
    assert!(after_safety_net.contains("## Previous Attempt Feedback"));
    assert!(after_safety_net.contains("use cargo before mise"));
}

#[test]
fn placeholder_empty_when_no_retry_no_visual_artifact() {
    let template = "intro {{retry_feedback_section}} outro";
    let section = format_retry_feedback_section(None);
    assert_eq!(section, "");
    let rendered = template.replace("{{retry_feedback_section}}", &section);
    assert_eq!(
        rendered, "intro  outro",
        "empty section must collapse cleanly"
    );
}
