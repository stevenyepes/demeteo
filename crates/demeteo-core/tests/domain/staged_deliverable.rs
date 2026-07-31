//! The stranded-deliverable guard, over the two lists the adapter already
//! holds. No git, no port, no runtime.

use super::*;

fn writes(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

/// Branch (b): the stage carries only report paths while the agent reported
/// writes outside them. The unambiguous case, and the only one that fails the
/// step.
///
/// The reason is asserted **byte for byte**, not with `contains`: it embeds
/// `{:?}` of a `Vec<&str>` and a `&[String]`, which happen to Debug-render
/// identically. A change to either container type — `Vec<&String>`, a
/// `BTreeSet` — would silently reword the retry feedback.
#[test]
fn only_report_paths_in_the_stage_strands_the_deliverable() {
    let verdict = judge_stage(
        &["artifacts/s-draft.md", "artifacts/s-notes.md"],
        &writes(&["docs/area/topic.md"]),
        "artifacts",
    );
    assert_eq!(
        verdict,
        StageVerdict::Stranded {
            reason: "agent stranded the deliverable under `artifacts` instead of writing it to \
                     the real repo path. Stage contains only artifact paths \
                     ([\"artifacts/s-draft.md\", \"artifacts/s-notes.md\"]) while the agent \
                     reported writes outside the report subdir ([\"docs/area/topic.md\"]). \
                     Re-read the survey's 'Files to Create' / 'Files to Update' sections and \
                     write the doc body to the real repo path (e.g. `docs/<area>/<topic>.md`), \
                     NOT to artifacts/s-*.md."
                .to_string()
        }
    );
}

/// Branch (a) is deliberately its own verdict and deliberately warn-only: the
/// cause is ambiguous — the deliverable could have been reverted by the
/// post-step diff guard, fenced out by the scope fence, or genuinely never
/// written — and all three surface earlier in the step executor. Promoting it
/// to a failure would double-report one root cause and confuse the retry loop's
/// feedback. Folding it into `Ok` would drop the only signal there is.
#[test]
fn an_empty_stage_with_reported_writes_is_its_own_verdict() {
    assert_eq!(
        judge_stage(&[], &writes(&["docs/new.md"]), "artifacts"),
        StageVerdict::EmptyStage
    );
}

#[test]
fn one_staged_path_outside_the_subdir_is_enough() {
    assert_eq!(
        judge_stage(
            &["artifacts/s-draft.md", "docs/area/topic.md"],
            &writes(&["docs/area/topic.md"]),
            "artifacts"
        ),
        StageVerdict::Ok
    );
}

#[test]
fn no_reported_writes_means_nothing_to_judge() {
    assert_eq!(
        judge_stage(&["artifacts/s-draft.md"], &[], "artifacts"),
        StageVerdict::Ok
    );
    assert_eq!(judge_stage(&[], &[], "artifacts"), StageVerdict::Ok);
}

/// An empty subdir disables the guard: `is_under_prefix` answers `false` for
/// every path, so the stage always looks like it holds a deliverable. Current
/// behaviour — a "defensive" flip would start failing steps.
#[test]
fn an_empty_subdir_disables_the_guard_rather_than_failing_everything() {
    assert_eq!(
        judge_stage(
            &["artifacts/s-draft.md"],
            &writes(&["docs/area/topic.md"]),
            ""
        ),
        StageVerdict::Ok
    );
}

// ── is_under_prefix ──────────────────────────────────────────────────────────

#[test]
fn nothing_is_under_an_empty_prefix() {
    assert!(!is_under_prefix("artifacts/x.md", ""));
    assert!(!is_under_prefix("", ""));
}

#[test]
fn the_directory_itself_counts_as_under_it() {
    assert!(is_under_prefix("artifacts", "artifacts"));
    assert!(is_under_prefix("artifacts/s-draft.md", "artifacts"));
}

/// A sibling whose name merely starts with the prefix is not inside it — the
/// separator is what makes it a directory match.
#[test]
fn a_name_sharing_the_prefix_is_not_inside_it() {
    assert!(!is_under_prefix("artifactsX/y.md", "artifacts"));
}

// ── normalize_artifact_subdir ────────────────────────────────────────────────

#[test]
fn the_subdir_normalises_to_its_pathspec_form() {
    assert_eq!(normalize_artifact_subdir("  ./artifacts/  "), "artifacts");
    assert_eq!(normalize_artifact_subdir("artifacts"), "artifacts");
    assert_eq!(normalize_artifact_subdir("artifacts/"), "artifacts");
    assert_eq!(normalize_artifact_subdir(""), "");
}
