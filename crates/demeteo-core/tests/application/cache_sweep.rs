use super::*;

fn entry(kind: SiblingKind, outcome: Option<Outcome>) -> SweepEntry {
    SweepEntry {
        path: "/w/repo_x".to_string(),
        kind,
        verdict: Verdict::Delete,
        reason: Reason::FeatureReleased.into(),
        outcome,
    }
}

fn project(entries: Vec<SweepEntry>) -> ProjectSweep {
    ProjectSweep {
        project_id: "p-1".to_string(),
        clone_dir: Some("/w/repo".to_string()),
        error: None,
        worktree_list_error: None,
        entries,
    }
}

#[test]
fn the_notice_counts_only_what_was_deleted() {
    let sweep = project(vec![
        entry(SiblingKind::Cache, Some(Outcome::Deleted)),
        entry(SiblingKind::Cache, Some(Outcome::Deleted)),
        entry(
            SiblingKind::Cache,
            Some(Outcome::Failed("busy".to_string())),
        ),
        entry(SiblingKind::Cache, None),
        entry(SiblingKind::Worktree, Some(Outcome::Deleted)),
    ]);

    assert_eq!(
        sweep.reclaimed_notice().as_deref(),
        Some("Reclaimed 2 dependency caches and 1 leaked worktree beside /w/repo")
    );
}

#[test]
fn a_sweep_that_deleted_nothing_says_nothing() {
    let dry = project(vec![
        entry(SiblingKind::Cache, None),
        entry(
            SiblingKind::Worktree,
            Some(Outcome::Spared(Reason::FeatureMayRun.into())),
        ),
    ]);

    assert_eq!(dry.reclaimed_notice(), None);
    assert_eq!(project(vec![]).reclaimed_notice(), None);
}
