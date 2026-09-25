//! Template selection and placeholder binding for a rework cycle.
//!
//! Both functions under test take exactly what they read — a `StepConfig`,
//! a mode, a retry context — so nothing here needs an `ExecutionDriver` or
//! its twenty-odd ports. `rework_mode` itself is the four-line forward to
//! `domain::rework::classify`, covered there.

use super::*;
use crate::domain::ids::StepId;

fn step(prompt: Option<&str>, rework: Option<&str>) -> StepConfig {
    StepConfig {
        id: StepId::from("tickets"),
        kind: "agent".to_string(),
        title: "Tickets".to_string(),
        prompt_template: prompt.map(str::to_string),
        rework_prompt_template: rework.map(str::to_string),
        ..Default::default()
    }
}

fn retry(files: &[&str], tests: &[&str]) -> RetryContext {
    RetryContext {
        feedback: "four criteria are not met".to_string(),
        iteration: 2,
        max: 3,
        failing_tests: tests.iter().map(|s| s.to_string()).collect(),
        implicated_files: files.iter().map(|s| s.to_string()).collect(),
        failing_step_id: "validate".to_string(),
    }
}

// ---------- effective_prompt_template ----------

#[test]
fn rework_mode_selects_the_rework_template() {
    let s = step(Some("greenfield"), Some("delta"));
    assert_eq!(effective_prompt_template(&s, ReworkMode::Rework), "delta");
}

#[test]
fn every_other_mode_keeps_the_ordinary_template() {
    let s = step(Some("greenfield"), Some("delta"));
    assert_eq!(
        effective_prompt_template(&s, ReworkMode::Greenfield),
        "greenfield"
    );
    assert_eq!(
        effective_prompt_template(&s, ReworkMode::Revision),
        "greenfield"
    );
}

#[test]
fn a_step_declaring_no_rework_template_is_unaffected() {
    // The whole back-compat claim: every workflow shipped before this
    // field existed renders exactly what it rendered before, in every
    // mode.
    let s = step(Some("greenfield"), None);
    for mode in [
        ReworkMode::Greenfield,
        ReworkMode::Revision,
        ReworkMode::Rework,
    ] {
        assert_eq!(effective_prompt_template(&s, mode), "greenfield");
    }
}

#[test]
fn a_blank_rework_template_falls_back_rather_than_rendering_nothing() {
    // An author who cleared the field in the builder leaves `Some("")`.
    // Honouring it would spawn an agent with an empty prompt.
    let s = step(Some("greenfield"), Some("   \n  "));
    assert_eq!(
        effective_prompt_template(&s, ReworkMode::Rework),
        "greenfield"
    );
}

#[test]
fn a_step_with_no_template_at_all_renders_empty() {
    let s = step(None, None);
    assert_eq!(effective_prompt_template(&s, ReworkMode::Rework), "");
}

// ---------- bind_rework_context ----------

const TEMPLATE: &str = "mode={{rework_mode}} origin={{retry_origin}} \
                        cycle={{rework_cycle}}\nfiles:\n{{implicated_files}}\n\
                        tests:\n{{failing_tests}}";

fn render(mode: ReworkMode, rc: Option<&RetryContext>) -> String {
    bind_rework_context(PromptContext::new(), mode, rc).render(TEMPLATE)
}

#[test]
fn a_verdicts_structured_lists_reach_the_prompt_as_bullets() {
    let rc = retry(
        &["src-tauri/src/domain/ai.rs", "pkg/arch/PKGBUILD"],
        &["ai::stream::tool_calls"],
    );
    let out = render(ReworkMode::Rework, Some(&rc));
    assert!(out.contains("mode=rework"), "{out}");
    assert!(out.contains("origin=validate"), "{out}");
    assert!(out.contains("cycle=2"), "{out}");
    assert!(out.contains("- src-tauri/src/domain/ai.rs"), "{out}");
    assert!(out.contains("- pkg/arch/PKGBUILD"), "{out}");
    assert!(out.contains("- ai::stream::tool_calls"), "{out}");
}

#[test]
fn a_fresh_run_renders_every_placeholder_empty() {
    // A template may reference these unconditionally, so the not-a-retry
    // case has to collapse to nothing rather than to a literal token or a
    // stray bullet.
    let out = render(ReworkMode::Greenfield, None);
    assert_eq!(
        out, "mode=greenfield origin= cycle=\nfiles:\n\ntests:\n",
        "{out}"
    );
}

#[test]
fn a_verdict_that_named_nothing_emits_no_bullet() {
    // `- ` on its own reads as one unnamed file, which is worse than
    // saying nothing.
    let rc = retry(&[], &["", "   "]);
    let out = render(ReworkMode::Rework, Some(&rc));
    assert!(!out.contains('-'), "expected no bullets, got: {out}");
}

#[test]
fn blank_entries_are_dropped_but_real_ones_survive() {
    let rc = retry(&["", "src/a.rs", "  "], &[]);
    let out = render(ReworkMode::Rework, Some(&rc));
    assert!(out.contains("- src/a.rs"), "{out}");
    assert_eq!(out.matches("- ").count(), 1, "{out}");
}

// ---------- persist_retry_context ----------

fn seeded_db() -> crate::adapters::database::SqliteAdapter {
    let db = crate::adapters::database::SqliteAdapter::new(
        rusqlite::Connection::open_in_memory().unwrap(),
    )
    .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0);
             INSERT INTO features (id, project_id, title, created_at)
             VALUES ('f-1', 'p-1', 'resume me', 0);",
        )
        .unwrap();
    db
}

/// An open loop is saved; closing it clears the row, so a restart after
/// the loop closed cannot reopen it.
#[test]
fn an_open_loop_is_saved_and_a_closed_one_cleared() {
    let db = seeded_db();
    let f = FeatureId::from("f-1".to_string());
    let rc = retry(&["src/lib.rs"], &["tests::a"]);

    persist_retry_context(&db, &f, Some(&rc));
    assert_eq!(db.retry_context_load(&f).unwrap(), Some(rc));

    persist_retry_context(&db, &f, None);
    assert_eq!(db.retry_context_load(&f).unwrap(), None);
}

/// A write the store refuses — here the foreign key, for a feature with no
/// row — is logged, not raised: the redirect it rides on must still happen.
#[test]
fn a_refused_write_does_not_fail_the_caller() {
    let db = seeded_db();
    let orphan = FeatureId::from("f-missing".to_string());
    assert!(db
        .retry_context_save(&orphan, &RetryContext::default(), 0)
        .is_err());
    persist_retry_context(&db, &orphan, Some(&RetryContext::default()));
}
