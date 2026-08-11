// `super` = `domain::artifact_capture`. No doubles and no runtime — both
// renderings are pure over what the step did and did not deliver.

use super::*;

fn missing(name: &str, detail: &str) -> MissingArtifact {
    MissingArtifact {
        name: name.to_string(),
        detail: detail.to_string(),
    }
}

#[test]
fn nothing_missing_leaves_the_reason_untouched() {
    // The common case, and the one that must stay byte-identical: this string
    // is retry feedback, and appending boilerplate to every verdict would
    // dilute the part the next agent has to act on.
    let reason = "criterion 3 not met: the debounce is missing";
    assert_eq!(note_undelivered_artifacts(reason, &[]), reason);
}

#[test]
fn a_missing_report_is_named_without_displacing_the_verdict() {
    let reason = "criterion 3 not met: the debounce is missing";
    let out = note_undelivered_artifacts(
        reason,
        &[missing(
            "validation-report",
            "artifacts/validation-report.md",
        )],
    );

    // The verdict leads. It is what the rework step decomposes into tickets;
    // the artifact note is context, not a replacement.
    assert!(out.starts_with(reason), "verdict must lead; got:\n{out}");
    assert!(out.contains("validation-report"));
    assert!(out.contains("artifacts/validation-report.md"));
}

#[test]
fn several_undelivered_artifacts_are_all_named() {
    let out = note_undelivered_artifacts(
        "rejected",
        &[
            missing("validation-report", "artifacts/validation-report.md"),
            missing("coverage", "artifacts/coverage.json"),
        ],
    );
    assert!(out.contains("validation-report"));
    assert!(out.contains("coverage"));
}

// ── missing_deliverables_message ────────────────────────────────────

#[test]
fn one_undelivered_artifact_is_singular() {
    let out = missing_deliverables_message(&[missing("plan", "artifacts/plan.md")]);
    assert!(
        out.contains("1 declared artifact was never produced"),
        "singular reads wrong in the plural: {out}"
    );
    assert!(!out.contains("declared artifacts were"));
}

#[test]
fn two_undelivered_artifacts_are_plural() {
    let out = missing_deliverables_message(&[
        missing("plan", "artifacts/plan.md"),
        missing("spec", "by name"),
    ]);
    assert!(
        out.contains("2 declared artifacts were never produced"),
        "plural reads wrong in the singular: {out}"
    );
    assert!(!out.contains("declared artifact was"));
}

#[test]
fn every_missing_name_and_detail_is_named() {
    let out = missing_deliverables_message(&[
        missing("plan", "artifacts/plan.md"),
        missing("spec", "by name"),
    ]);
    for token in ["'plan'", "artifacts/plan.md", "'spec'", "by name"] {
        assert!(out.contains(token), "missing `{token}` in: {out}");
    }
}

#[test]
fn the_message_names_the_causes_a_user_can_act_on() {
    let out = missing_deliverables_message(&[missing("plan", "artifacts/plan.md")]);
    for cause in ["failed", "different path", "model/config", "opencode.json"] {
        assert!(
            out.contains(cause),
            "`{cause}` is one of the four named causes of this failure class: {out}"
        );
    }
    assert!(out.contains("Nothing downstream can consume this step."));
}

// ── the two renderings are deliberately different ───────────────────

#[test]
fn the_two_renderings_stay_distinct() {
    let one = [missing("report", "artifacts/report.md")];
    let step_failure = missing_deliverables_message(&one);
    let verdict_note = note_undelivered_artifacts("rejected", &one);

    assert!(
        step_failure.starts_with("The step completed but"),
        "the step-failure wording is what the UI renders on the failed row"
    );
    assert!(
        verdict_note.starts_with("rejected"),
        "the verdict note is appended to feedback a downstream step reads"
    );
    assert_ne!(
        step_failure, verdict_note,
        "unifying the two is a behaviour change, not a cleanup"
    );
}

// ── One declaration against one turn's output ────────────────────────────────
//
// Absorbed from `tests/infrastructure/step_executor/artifacts/declared.rs`,
// which reached these five answers through an `FsArtifactStore` in a temp
// directory. The store never participated in the decision; it only recorded it.

use crate::domain::artifact::{Artifact, ArtifactCapture, ArtifactDecl, ArtifactMode, DiffBase};

fn decl(name: &str, capture: ArtifactCapture) -> ArtifactDecl {
    ArtifactDecl {
        name: name.to_string(),
        capture,
        mode: ArtifactMode::Full,
        inline: false,
    }
}

#[test]
fn a_by_name_declaration_matches_the_artifact_the_agent_named() {
    let produced = vec![Artifact::tool_write("spec", "docs/spec.md", "# My Spec\n")];
    let d = decl(
        "spec",
        ArtifactCapture::ByName {
            name: "spec".to_string(),
        },
    );
    let CaptureOutcome::Store(a) = resolve_capture(&d, &produced) else {
        panic!("a matched declaration is not missing");
    };
    assert_eq!(a.content, "# My Spec\n");
}

/// `Path::file_stem`, not `rsplit_once('.')`: a produced `a/b.md` answers a
/// declaration for `b`.
#[test]
fn a_by_name_declaration_also_matches_a_stem() {
    let produced = vec![Artifact::tool_write("a/b.md", "a/b.md", "body\n")];
    let d = decl(
        "b",
        ArtifactCapture::ByName {
            name: "b".to_string(),
        },
    );
    assert!(matches!(
        resolve_capture(&d, &produced),
        CaptureOutcome::Store(_)
    ));
}

#[test]
fn last_write_to_keeps_the_last_write_not_the_first() {
    let produced = vec![
        Artifact::tool_write("draft", "docs/spec.md", "# Draft\n"),
        Artifact::tool_write("final", "docs/spec.md", "# Final\n"),
    ];
    let d = ArtifactDecl::full_path("final-spec", "docs/spec.md");
    let CaptureOutcome::Store(a) = resolve_capture(&d, &produced) else {
        panic!("a matched declaration is not missing");
    };
    assert_eq!(
        a.content, "# Final\n",
        "the agent's final version of a file it revised, not its first draft"
    );
}

#[test]
fn all_writes_keeps_the_first_artifact_per_path() {
    let produced = vec![
        Artifact::tool_write("f1", "src/lib.rs", "// lib\n"),
        Artifact::tool_write("f2", "src/main.rs", "// main\n"),
        Artifact::tool_write("f1-v2", "src/lib.rs", "// lib v2\n"),
    ];
    let decls = vec![decl("all-files", ArtifactCapture::AllWrites)];

    assert!(
        matches!(
            resolve_capture(&decls[0], &produced),
            CaptureOutcome::Skip(None)
        ),
        "AllWrites never yields a missing deliverable"
    );
    let selected = all_writes_selection(&decls, &produced);
    assert_eq!(
        selected.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        vec!["f1", "f2"],
        "the opposite end from LastWriteTo: first write per path, in turn order"
    );
}

#[test]
fn all_writes_selects_nothing_when_no_declaration_asks_for_it() {
    let produced = vec![Artifact::tool_write("f1", "src/lib.rs", "// lib\n")];
    let decls = vec![ArtifactDecl::full_path("spec", "docs/spec.md")];
    assert!(all_writes_selection(&decls, &produced).is_empty());
}

#[test]
fn diff_and_worktree_are_skipped_and_never_counted_as_missing() {
    let diff = decl(
        "code-diff",
        ArtifactCapture::Diff {
            base: DiffBase::WorktreeBase,
            path_filter: None,
        },
    );
    let wt = decl("wt-ref", ArtifactCapture::Worktree { path: None });

    assert!(matches!(
        resolve_capture(&diff, &[]),
        CaptureOutcome::Skip(Some(UnwiredCapture::Diff))
    ));
    assert!(matches!(
        resolve_capture(&wt, &[]),
        CaptureOutcome::Skip(Some(UnwiredCapture::Worktree))
    ));
}

/// A `LastWriteTo` deliverable the agent never wrote is reported as missing
/// (not silently skipped) so the step executor fails the step instead of
/// marking it `completed` with an empty artifact — the "green step, no plan
/// produced" misconfiguration class.
#[test]
fn an_unwritten_deliverable_is_missing_and_names_the_path_it_expected() {
    let produced = vec![Artifact::tool_write(
        "notes",
        "scratch/notes.md",
        "# notes\n",
    )];
    let d = ArtifactDecl::full_path("plan", "artifacts/plan.md");
    let CaptureOutcome::Missing(m) = resolve_capture(&d, &produced) else {
        panic!("the unmatched plan deliverable is missing");
    };
    assert_eq!(m.name, "plan");
    assert_eq!(m.detail, "expected a write to `artifacts/plan.md`");
}

#[test]
fn an_unproduced_named_artifact_names_the_name_it_expected() {
    let d = decl(
        "review",
        ArtifactCapture::ByName {
            name: "x".to_string(),
        },
    );
    let CaptureOutcome::Missing(m) = resolve_capture(&d, &[]) else {
        panic!("an unmatched ByName declaration is missing");
    };
    assert_eq!(m.detail, "expected an artifact named `x`");
}

/// The `_ =>` arm is unreachable — every other capture returns before it — but
/// it is kept total so a future capture kind is handled rather than compiled
/// into a panic. This pins what it would say.
#[test]
fn the_total_fallback_detail_is_the_generic_wording() {
    let d = decl(
        "changed",
        ArtifactCapture::ChangedFiles {
            base: DiffBase::WorktreeBase,
            path_filter: None,
        },
    );
    assert!(matches!(
        resolve_capture(&d, &[]),
        CaptureOutcome::Skip(None)
    ));
}

/// The four captures that consume a `ToolWrite` body, stated one at a time so
/// a new capture kind added to the wrong side of the split shows up here and
/// not as a step that silently stopped delivering its artifact.
#[test]
fn every_body_consuming_capture_asks_for_the_reads() {
    for capture in [
        ArtifactCapture::AllWrites,
        ArtifactCapture::ChangedFiles {
            base: DiffBase::WorktreeBase,
            path_filter: None,
        },
        ArtifactCapture::ByName {
            name: "spec".into(),
        },
        ArtifactCapture::LastWriteTo {
            path: "docs/spec.md".into(),
        },
    ] {
        assert!(
            captures_file_bodies(&[decl("d", capture.clone())]),
            "{capture:?} is satisfied from a file body, so the reads must happen"
        );
    }
}

/// The captures derived from git and branch state, plus the step that declares
/// nothing. This is the case the whole predicate exists for: a scaffolding
/// step whose every changed file would otherwise be read back and dropped.
#[test]
fn a_git_derived_or_empty_declaration_reads_nothing() {
    assert!(
        !captures_file_bodies(&[]),
        "a step declaring nothing has nothing to satisfy from a body"
    );
    for capture in [
        ArtifactCapture::Diff {
            base: DiffBase::WorktreeBase,
            path_filter: None,
        },
        ArtifactCapture::Worktree { path: None },
    ] {
        assert!(
            !captures_file_bodies(&[decl("d", capture.clone())]),
            "{capture:?} is derived, not read"
        );
    }
}

/// One body-consuming declaration among derived ones still buys the reads —
/// the predicate is an `any`, and a step that mixes them must not lose the
/// half that needs them.
#[test]
fn one_body_consumer_among_derived_declarations_is_enough() {
    assert!(captures_file_bodies(&[
        decl(
            "diff",
            ArtifactCapture::Diff {
                base: DiffBase::WorktreeBase,
                path_filter: None,
            },
        ),
        decl(
            "report",
            ArtifactCapture::LastWriteTo {
                path: "r.md".into()
            }
        ),
    ]));
}
