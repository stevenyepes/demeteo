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

/// Pinning the harness alone must not drag a lower tier's model up with it:
/// `sonnet` is opencode's name for a model and means nothing to codex, so the
/// spawn that came out of this was `codex --model sonnet`. Effort has no such
/// namespace and still crosses.
#[test]
fn a_pinned_harness_leaves_the_lower_tiers_model_behind() {
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
    assert_eq!(resolved.model, None);
    assert_eq!(resolved.effort, EffortLevel::Medium);
}

/// Two tiers that name the same harness still hand the model across it.
#[test]
fn a_model_crosses_a_tier_that_names_the_same_harness() {
    let silent = SyncResolverChoice::default();
    let asked = choice(Some("codex"), None, Some(EffortLevel::High));
    let project_sync = choice(Some("codex"), Some("gpt-5-codex"), None);
    let resolved = SyncResolverChain {
        asked: &asked,
        project_sync: &project_sync,
        run: &silent,
        project_default: &silent,
    }
    .resolve();

    assert_eq!(resolved.agent_kind, "codex");
    assert_eq!(resolved.model.as_deref(), Some("gpt-5-codex"));
}

/// The tier holding the model names no harness at all, so which one it meant is
/// whatever the tiers *below* it resolve to — codex here, the same harness the
/// pin above names. Folded the other way this model is dropped against a `None`
/// that was never a disagreement.
#[test]
fn a_model_crosses_a_tier_whose_harness_comes_from_below() {
    let asked = choice(None, None, Some(EffortLevel::Low));
    let project_sync = choice(Some("codex"), None, None);
    let run = choice(None, Some("gpt-5-codex"), Some(EffortLevel::Max));
    let project_default = choice(Some("codex"), None, None);
    let resolved = SyncResolverChain {
        asked: &asked,
        project_sync: &project_sync,
        run: &run,
        project_default: &project_default,
    }
    .resolve();

    assert_eq!(resolved.agent_kind, "codex");
    assert_eq!(resolved.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(resolved.effort, EffortLevel::Low);
}

/// …and the same shape with a different harness underneath keeps that model out
/// of the spawn: the run's model was chosen for opencode.
#[test]
fn a_model_stops_at_a_tier_whose_harness_comes_from_below() {
    let silent = SyncResolverChoice::default();
    let project_sync = choice(Some("codex"), None, None);
    let run = choice(None, Some("sonnet"), None);
    let project_default = choice(Some("opencode"), None, None);
    let resolved = SyncResolverChain {
        asked: &silent,
        project_sync: &project_sync,
        run: &run,
        project_default: &project_default,
    }
    .resolve();

    assert_eq!(resolved.agent_kind, "codex");
    assert_eq!(resolved.model, None);
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

mod sync_node {
    use super::*;
    use crate::domain::ids::StepId;
    use crate::domain::models::{StepConfig, StepOverride};

    fn settings(sync: SyncResolverChoice) -> crate::domain::models::ProjectSettings {
        let mut s = crate::adapters::step_executor::setup::fetch_default_settings();
        s.sync_resolver_agent_kind = sync.agent_kind;
        s.sync_resolver_model = sync.model;
        s.sync_resolver_effort = sync.effort;
        s
    }

    fn node(agent: Option<&str>, model: Option<&str>, effort: Option<EffortLevel>) -> StepConfig {
        StepConfig {
            id: StepId::from("s-sync".to_string()),
            kind: "sync".to_string(),
            agent_kind: agent.map(str::to_string),
            model: model.map(str::to_string),
            effort,
            ..Default::default()
        }
    }

    fn resolve(
        step_conf: &StepConfig,
        step_override: Option<&StepOverride>,
        stored: crate::domain::models::ProjectSettings,
        run: SyncResolverChoice,
    ) -> SyncResolver {
        SyncNodeTiers {
            step_conf,
            step_override,
            settings: &stored,
            run: &run,
            project_default: &SyncResolverChoice::default(),
        }
        .resolve()
    }

    /// The node's own config is the tier the button's picker fills, so it
    /// outranks the harness the run was launched with — and its `model`, which
    /// the node's JSON schema has always advertised, reaches the spawn.
    #[test]
    fn the_nodes_own_config_outranks_the_runs_launch_pin() {
        let resolved = resolve(
            &node(Some("codex"), Some("gpt-5-codex"), Some(EffortLevel::Low)),
            None,
            settings(SyncResolverChoice::default()),
            choice(Some("opencode"), Some("sonnet"), Some(EffortLevel::Max)),
        );

        assert_eq!(resolved.agent_kind, "codex");
        assert_eq!(resolved.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(resolved.effort, EffortLevel::Low);
    }

    /// This run's override for this node beats what the workflow declared, and
    /// pinning only the harness in it leaves the node's model behind — that
    /// model named a model of the workflow's harness.
    #[test]
    fn a_per_step_run_override_outranks_the_node() {
        let ov = StepOverride {
            step_id: "s-sync".to_string(),
            agent_kind: Some("pi".to_string()),
            model: None,
            effort: None,
        };
        let resolved = resolve(
            &node(Some("codex"), Some("gpt-5-codex"), Some(EffortLevel::Low)),
            Some(&ov),
            settings(SyncResolverChoice::default()),
            SyncResolverChoice::default(),
        );

        assert_eq!(resolved.agent_kind, "pi");
        assert_eq!(resolved.model, None);
        assert_eq!(resolved.effort, EffortLevel::Low);
    }

    /// A node that declares nothing lands on the project's conflict resolver,
    /// which is what makes the setting reach an in-workflow sync at all and not
    /// only the button.
    #[test]
    fn a_bare_node_lands_on_the_projects_conflict_resolver() {
        let resolved = resolve(
            &node(None, None, None),
            None,
            settings(choice(
                Some("codex"),
                Some("gpt-5-codex"),
                Some(EffortLevel::Low),
            )),
            choice(Some("opencode"), Some("sonnet"), Some(EffortLevel::Max)),
        );

        assert_eq!(resolved.agent_kind, "codex");
        assert_eq!(resolved.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(resolved.effort, EffortLevel::Low);
    }

    /// With nothing configured anywhere the node runs what both sync paths
    /// terminated at before there was a chain.
    #[test]
    fn a_bare_node_in_a_bare_project_still_terminates_at_opencode() {
        let resolved = resolve(
            &node(None, None, None),
            None,
            settings(SyncResolverChoice::default()),
            SyncResolverChoice::default(),
        );

        assert_eq!(resolved.agent_kind, "opencode");
        assert_eq!(resolved.model, None);
        assert_eq!(resolved.effort, EffortLevel::DEFAULT);
    }
}
