use super::*;
use crate::domain::models::EffortLevel;

fn choice(
    agent: Option<&str>,
    model: Option<&str>,
    effort: Option<EffortLevel>,
) -> SyncResolverChoice {
    SyncResolverChoice {
        agent_kind: agent.map(str::to_string),
        model: model.map(str::to_string),
        effort,
    }
}

/// All four tiers name a harness; the highest one must win, and each tier in
/// turn once the ones above it fall silent.
#[test]
fn each_tier_outranks_the_next() {
    let asked = choice(Some("codex"), None, None);
    let project_sync = choice(Some("hermes"), None, None);
    let run = choice(Some("claude-code"), None, None);
    let project_default = choice(Some("pi"), None, None);

    let resolve = |asked: &SyncResolverChoice| {
        SyncResolverChain {
            asked,
            project_sync: &project_sync,
            run: &run,
            project_default: &project_default,
        }
        .resolve()
        .agent_kind
    };

    assert_eq!(resolve(&asked), "codex");
    assert_eq!(resolve(&SyncResolverChoice::default()), "hermes");

    let silent = SyncResolverChoice::default();
    assert_eq!(
        SyncResolverChain {
            asked: &silent,
            project_sync: &silent,
            run: &run,
            project_default: &project_default,
        }
        .resolve()
        .agent_kind,
        "claude-code"
    );
    assert_eq!(
        SyncResolverChain {
            asked: &silent,
            project_sync: &silent,
            run: &silent,
            project_default: &project_default,
        }
        .resolve()
        .agent_kind,
        "pi"
    );
}

/// A conflict-resolver default set on the project beats the harness the run was
/// launched with — the tier order this module exists to state.
#[test]
fn project_sync_default_beats_the_runs_launch_pin() {
    let silent = SyncResolverChoice::default();
    let project_sync = choice(Some("codex"), Some("gpt-5"), Some(EffortLevel::Low));
    let run = choice(Some("opencode"), Some("sonnet"), Some(EffortLevel::Max));
    let resolved = SyncResolverChain {
        asked: &silent,
        project_sync: &project_sync,
        run: &run,
        project_default: &silent,
    }
    .resolve();

    assert_eq!(resolved.agent_kind, "codex");
    assert_eq!(resolved.model.as_deref(), Some("gpt-5"));
    assert_eq!(resolved.effort, EffortLevel::Low);
}

/// The harness and the model may be pinned at different tiers; nothing here
/// takes a whole tier at once.
#[test]
fn dimensions_resolve_independently() {
    let silent = SyncResolverChoice::default();
    let asked = choice(Some("codex"), None, None);
    let run = choice(Some("opencode"), Some("sonnet"), Some(EffortLevel::Medium));
    let resolved = SyncResolverChain {
        asked: &asked,
        project_sync: &silent,
        run: &run,
        project_default: &silent,
    }
    .resolve();

    assert_eq!(resolved.agent_kind, "codex");
    assert_eq!(resolved.model.as_deref(), Some("sonnet"));
    assert_eq!(resolved.effort, EffortLevel::Medium);
}

/// A chain nobody has an opinion in resolves to what both sync paths already
/// terminated at, so a project that has configured nothing is unaffected.
#[test]
fn a_silent_chain_terminates_at_opencode_and_the_default_effort() {
    let silent = SyncResolverChoice::default();
    let resolved = SyncResolverChain {
        asked: &silent,
        project_sync: &silent,
        run: &silent,
        project_default: &silent,
    }
    .resolve();

    assert_eq!(resolved.agent_kind, "opencode");
    assert_eq!(resolved.model, None);
    assert_eq!(resolved.effort, EffortLevel::DEFAULT);
}

/// A harness Demeteo has no runtime for is the one thing a choice can get
/// wrong, and naming none is not it.
#[test]
fn only_a_named_unknown_harness_is_refused() {
    assert_eq!(
        choice(Some("antigravity"), None, None).unsupported_agent_kind(),
        Some("antigravity")
    );
    assert_eq!(
        choice(Some("codex"), None, None).unsupported_agent_kind(),
        None
    );
    assert_eq!(SyncResolverChoice::default().unsupported_agent_kind(), None);
}
