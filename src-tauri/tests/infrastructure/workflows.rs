// Tests extracted from `src-tauri/src/commands/workflows.rs` (mirrored-tests convention). `super` = that module.

use crate::domain::models::StepConfig;
use crate::domain::permission::StepCapability;

use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::{WorkflowId, WorkflowVersionId};
use crate::domain::models::{Workflow, WorkflowVersion};
use crate::domain::workflow_starters::{plan_seed, SeedAction, StarterDefinition};
use crate::ports::db::WorkflowRepository;
use rusqlite::Connection;
use std::sync::Arc;

const CODE_REVIEW: &str = include_str!("../../workflows/code-review.json");
const ADDRESS_REVIEW: &str = include_str!("../../workflows/address-review.json");

/// Every embedded starter workflow must deserialize into `StepConfig`
/// (guards the V13 `model`/`verifier` JSON edits against typos).
fn parse(json: &str) -> Vec<StepConfig> {
    let v: serde_json::Value = serde_json::from_str(json).expect("starter JSON parses");
    serde_json::from_value(v["steps"].clone()).expect("steps deserialize into StepConfig")
}

/// Every prompt a starter ships, lowercased, for the vocabulary assertions
/// below to read as one body of text.
fn prompt_text(json: &str) -> String {
    let prompts = parse(json)
        .iter()
        .filter_map(|s| s.prompt_template.clone())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(!prompts.is_empty(), "read no prompt text at all");
    prompts
}

/// Reads `super::STARTER_FILES` — the list seeding actually walks — so a
/// starter that stops being registered fails here rather than shipping as a
/// file nobody loads.
#[test]
fn all_starters_deserialize() {
    for json in super::STARTER_FILES {
        let steps = parse(json);
        assert!(!steps.is_empty());
    }
}

/// The six looping workflows must each have a step that both redirects on
/// failure AND carries a verifier (the harness + agent-judgment gate).
#[test]
fn looping_starters_have_verifier_and_redirect() {
    for json in [
        include_str!("../../workflows/standard-feature-pipeline.json"),
        include_str!("../../workflows/bugfix-pipeline.json"),
        include_str!("../../workflows/docs-update.json"),
        include_str!("../../workflows/refactor.json"),
        include_str!("../../workflows/ci-fix.json"),
        include_str!("../../workflows/simple-task.json"),
    ] {
        let steps = parse(json);
        let has_loop = steps
            .iter()
            .any(|s| s.on_failure.is_some() && s.verifier.is_some());
        assert!(
            has_loop,
            "expected a validate step with on_failure + verifier"
        );
    }
}

/// The review starter ships no method of its own — no rubric, no severity
/// scale, no criteria. A review's depth is meant to come from the agent's own
/// review skill and from what the project wrote down about itself, and a
/// prompt that grew a rubric would run exactly as well as one that did not, so
/// nothing but this assertion holds the decision in place.
#[test]
fn the_review_starter_imposes_no_method_of_its_own() {
    let prompts = prompt_text(CODE_REVIEW);

    for phrase in [
        "critical",
        "major",
        "minor",
        "blocker",
        "check for",
        "look for",
        "ensure that",
        "verify that",
    ] {
        assert!(
            !prompts.contains(phrase),
            "the review prompt says '{phrase}' — Demeteo is imposing a review method"
        );
    }
}

/// `s-review` told the reviewer the workflow had one step and that nothing
/// would follow it. That stopped being true when `s-validate-branch` landed,
/// and the audience it misinformed is the one that cannot check it: a
/// reviewer reads the prompt as the description of the run it is inside.
/// Nothing compiles a `prompt_template` — a prompt describing a graph that
/// does not exist parses, lints and runs exactly as well as one describing
/// the real one — so the sentence survived every full `npm run checks` there
/// has been. What the assertion rejects is the *count* and the claim that the
/// run ends there, not any mention of what comes next: a reviewer who knows
/// the project's own gates run after it does not have to guess at build state,
/// and that sentence stays true however long the list grows.
#[test]
fn the_review_starter_does_not_describe_its_own_step_list() {
    let prompts = prompt_text(CODE_REVIEW);

    for claim in [
        "one step and nothing after it",
        "this workflow has one step",
    ] {
        assert!(
            !prompts.contains(claim),
            "the review prompt says '{claim}' — it is describing a graph it does not hold"
        );
    }
}

/// The engine already subtracts a gate that was red at the base commit with the
/// identical output, and says so in the Harness Results block this step reads
/// (`build_exclusion_reason`). Neither of this step's two prose fields had a
/// word for that subtraction, and a judge with no word for it has only one
/// thing left to call a red gate: a defect in the change under review. That is
/// how a branch which arrived red is written up as the pull request's fault.
///
/// Both fields are read because they are two different audiences — the step
/// writes the artifact, the verifier grades it — and a term present in only one
/// of them leaves the other free to contradict it. `prompt_text` reads
/// `prompt_template` alone, so it cannot serve here.
#[test]
fn the_review_starter_can_name_a_failure_it_did_not_cause() {
    let steps = parse(CODE_REVIEW);
    let gate = steps
        .iter()
        .find(|s| s.verifier.is_some())
        .expect("the review starter has a step the engine runs the project's gates for");

    let fields = [
        ("prompt_template", gate.prompt_template.clone()),
        (
            "verifier.instructions",
            gate.verifier.as_ref().map(|v| v.instructions.clone()),
        ),
    ];
    for (field, text) in fields {
        let text = text.unwrap_or_default().to_lowercase();
        assert!(
            text.contains("pre-existing"),
            "'{}' has no term in its {field} for a failure that pre-dates the branch, \
             so a gate that was already red is reported as this change's defect",
            gate.id.0
        );
    }
}

/// A review whose report can come back clean on a branch that does not build
/// is the bug this step exists to close. `run_harness_first` — the only code
/// that executes a project's `prepare_command` / `test_command` — is gated on
/// the step declaring a `verifier`, so that block is not decoration: it is the
/// whole of what makes the engine run this project's own gates before the turn.
/// Delete it from the starter and the file still parses, still lints, still
/// runs, and silently stops measuring anything.
#[test]
fn the_review_starter_runs_the_projects_own_gates() {
    let steps = parse(CODE_REVIEW);
    assert!(
        steps
            .iter()
            .any(|s| s.verifier.is_some() && s.effective_capability() == StepCapability::Verify),
        "no step asks the engine to run the project's gates, so a clean report \
         says nothing about whether the branch builds"
    );
}

fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.lines().flat_map(|line| line.split(". "))
}

/// A step's two prose fields, labelled for an assertion message. A step with
/// no verifier has no second one.
fn prose_fields(step: &StepConfig) -> [(&'static str, Option<&str>); 2] {
    [
        ("prompt_template", step.prompt_template.as_deref()),
        (
            "verifier.instructions",
            step.verifier.as_ref().map(|v| v.instructions.as_str()),
        ),
    ]
}

/// The gates run before `s-validate-branch`'s turn, not during it (see
/// `the_review_starter_runs_the_projects_own_gates`), and a gate this branch
/// turned red fails the step right there: no agent runs, `branch-validation.md`
/// is never written, and what survives is the failing command's output as the
/// step's failure reason. All three prose fields once promised that report
/// regardless, and told the verifier to grade a red gate it is never shown —
/// so a reviewer counted on an artifact the run could not produce, and nothing
/// but prose disagreed with the engine.
///
/// Every field of every step is read, because the promise was spread across
/// both audiences and either one alone would reinstate it.
#[test]
fn the_review_starter_does_not_promise_a_gate_report_on_a_red_branch() {
    let steps = parse(CODE_REVIEW);

    for step in &steps {
        for (field, text) in prose_fields(step) {
            let text = text.unwrap_or_default().to_lowercase();
            for promise in [
                "writes their output to",
                "when a listed command exited non-zero",
            ] {
                assert!(
                    !text.contains(promise),
                    "'{}' says '{promise}' in its {field} — a red gate ends the step \
                     before any agent turn, so nothing is there to write or grade it",
                    step.id.0
                );
            }
        }
    }

    for (id, field) in [
        ("s-review", "prompt_template"),
        ("s-validate-branch", "prompt_template"),
        ("s-validate-branch", "verifier.instructions"),
    ] {
        let step = steps
            .iter()
            .find(|s| s.id.0 == id)
            .unwrap_or_else(|| panic!("the review starter has no step '{id}'"));
        let text = prose_fields(step)
            .into_iter()
            .find(|(name, _)| *name == field)
            .and_then(|(_, text)| text)
            .unwrap_or_else(|| panic!("'{id}' has no {field}"))
            .to_lowercase();
        assert!(
            text.contains("ends the step before"),
            "'{id}' never says in its {field} that a red gate ends the step before \
             any agent turn, so its reader expects a report a red branch cannot produce"
        );
    }
}

/// A project that configures no gate leaves `s-validate-branch` with nothing
/// to run, and no review step may ask for `environment` there. The engine
/// reads that verdict as `VerdictDisposition::Unjudgeable`, which ends the step
/// without recording its artifact and fails the feature: every review of an
/// unconfigured project would end `failed`, its `branch-validation.md` written
/// but shown nowhere, and a fix launched from it would be told a gate failed
/// when none ran. The honest answer is `pass` with a report that leaves the
/// branch unjudged, which is what the step's own prompt asks the turn to write.
///
/// The starter still has to *name* `environment`, because the engine advises
/// it for this case and the override must say which advice it cancels. So the
/// ban is on asking, not on mentioning: the double-quoted literal — the value
/// a verdict JSON would carry — never appears, and every sentence that
/// mentions `` `environment` `` also negates it. Whether the override actually
/// lands after the engine's advice in the prompt the turn receives is held in
/// `crates/demeteo-core/tests/infrastructure/step_executor/steps/agent/prompt.rs`,
/// which renders it; this test only sees the JSON.
///
/// A gate that cannot start (exit 127, a dead transport, a timeout) never
/// reaches the verifier — harness-first ends it as a terminal `Environment`
/// before the turn — so no case is left for which `environment` is right here.
/// That the engine completes a `pass` with nothing configured is proved in
/// `crates/demeteo-core/tests/conformance/starter_baseline.rs`.
#[test]
fn the_review_starter_passes_a_branch_no_gate_was_configured_for() {
    let steps = parse(CODE_REVIEW);

    for step in &steps {
        for (field, text) in prose_fields(step) {
            let text = text.unwrap_or_default().to_lowercase();
            assert!(
                !text.contains("\"environment\""),
                "'{}' names the verdict \"environment\" in its {field}; for a review \
                 step it fails the feature and drops the step's report",
                step.id.0
            );
            for sentence in sentences(&text).filter(|s| s.contains("`environment`")) {
                assert!(
                    sentence
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|word| word == "not" || word == "never"),
                    "'{}' mentions `environment` in its {field} without negating it, \
                     so it reads as asking for the verdict that fails a review: {sentence}",
                    step.id.0
                );
            }
        }
    }

    let instructions = steps
        .iter()
        .find(|s| s.id.0 == "s-validate-branch")
        .and_then(|s| s.verifier.as_ref())
        .map(|v| v.instructions.to_lowercase())
        .expect("the review starter's gate step has verifier instructions");
    assert!(
        sentences(&instructions)
            .any(|sentence| sentence.contains("\"pass\"") && sentence.contains("no command")),
        "s-validate-branch's verifier is never told to return \"pass\" when no \
         command ran, so an unconfigured project's review has no verdict that completes"
    );
}

/// Every test-gated starter not cut from a pull request opens on
/// `s-baseline-harness`, and the two that are not read as an oversight to
/// anyone who counts them. It is the opposite. `run_baseline_node` states its own precondition: the head of the
/// graph is the one position where the feature branch still points at the base
/// commit, because nothing has been implemented yet. That is true of a run cut
/// from a default branch or from a branch, and false of a review — a review is
/// cut at the pull request's head, so a measurement taken there records the sha
/// of the tree under review, while the subtraction resolves its base through
/// `merge_base`. `HarnessBaseline::covers` is sha-exact, so those two never
/// meet: the record would be written, never matched, and the lazy fallback
/// would measure again anyway — one extra `prepare_command` and gate run per
/// review, for a record nothing reads.
///
/// Teaching the node to resolve its own base would close that, and it changes a
/// documented invariant across all nine starters, so it is a decision rather
/// than a detail. Until it is taken, the absence here is the configuration that
/// is correct, and nothing but this assertion says so.
#[test]
fn the_review_starter_measures_no_baseline_of_its_own() {
    let steps = parse(CODE_REVIEW);
    assert!(
        steps.iter().all(|s| s.measure_baseline != Some(true)),
        "a review branch is cut at the pull request's head, so a baseline taken \
         at the head of this graph names the tree under review rather than the \
         base it is judged against, and no subtraction can ever match it"
    );
}

/// `finalize` squashes the branch and hands it to the publisher, so a review
/// workflow that has one can ship the work it was asked to judge. Its absence
/// is the enforcement: a flag would be something a later edit could set back.
#[test]
fn the_review_starter_has_no_step_that_publishes() {
    let steps = parse(CODE_REVIEW);
    assert!(
        steps.iter().all(|s| s.kind != "finalize"),
        "a review must never commit, push or open a PR"
    );
}

/// The fix starter is the one place Demeteo *may* direct the work — it is
/// implementing, not judging — so the review starter's guard narrows here to
/// the vocabulary of judgement rather than the vocabulary of instruction. A
/// prompt that grades the findings it was handed is running the review a
/// second time, against a rubric this project never agreed to, and the
/// reviewer's own words are what the pull request will be read beside.
#[test]
fn the_fix_starter_directs_the_work_without_re_reviewing_it() {
    let prompts = prompt_text(ADDRESS_REVIEW);

    for phrase in [
        "critical",
        "major",
        "minor",
        "blocker",
        "severity",
        "code smell",
        "check for",
        "look for",
    ] {
        assert!(
            !prompts.contains(phrase),
            "the fix prompt says '{phrase}' — it is reviewing, not addressing a review"
        );
    }
}

/// The counterpart of [`the_review_starter_has_no_step_that_publishes`]: a run
/// that ends in commits has to put them somewhere a human can merge, and
/// `finalize` is what squashes the branch and hands it to the publisher.
/// Without it the work ends in a worktree nobody is told about.
#[test]
fn the_fix_starter_ends_in_a_pull_request() {
    let steps = parse(ADDRESS_REVIEW);
    assert!(
        steps.iter().any(|s| s.kind == "finalize"),
        "a fix run must end somewhere its commits can be merged from"
    );

    // Parsing the file proves nothing about the user ever seeing it: seeding
    // and `workflow_revert_to_default` both walk `STARTER_FILES`, so the
    // registration is the line that ships the workflow.
    assert!(
        crate::domain::workflow_starters::find(
            super::STARTER_FILES,
            &WorkflowId::from("wf-starter-address-review".to_string())
        )
        .is_some(),
        "the fix starter parses but is not registered, so nothing seeds it"
    );
}

/// `s-address` ships a `{{retry_feedback_section}}` block and the budget that
/// would fill it. Both are inert unless some step redirects back to it —
/// `legacy_policy_for_step` reads `max_iterations` only off a step whose
/// `on_failure` names a target — so the prompt would describe a retry cycle
/// that cannot happen.
#[test]
fn the_fix_starter_can_actually_retry_the_step_whose_prompt_says_it_will() {
    let steps = parse(ADDRESS_REVIEW);
    let address = steps
        .iter()
        .find(|s| s.id.0 == "s-address")
        .expect("the fix starter has an address step");
    assert!(
        address
            .prompt_template
            .as_deref()
            .is_some_and(|p| p.contains("{{retry_feedback_section}}")),
        "this test exists for that block; drop the test with it"
    );

    let redirect = steps
        .iter()
        .find(|s| s.on_failure.as_ref().is_some_and(|t| t.0 == "s-address"))
        .expect("nothing sends the run back to s-address, so its retry block is dead text");
    assert!(
        redirect.verifier.is_some(),
        "a redirect with no verifier never fires: nothing judges the step's own claim that it passed"
    );
    assert!(
        redirect.max_iterations.is_some(),
        "the redirect carries the budget, so it is the step that needs one"
    );
}

/// The only publishing starter in the pack with neither a gate nor a verifier
/// would open a pull request in the user's name on the agent's own say-so.
#[test]
fn the_fix_starter_checks_the_work_before_publishing_it() {
    let steps = parse(ADDRESS_REVIEW);
    let finalize = steps
        .iter()
        .position(|s| s.kind == "finalize")
        .expect("the fix starter finalizes");
    assert!(
        steps[..finalize]
            .iter()
            .any(|s| s.kind == "gate" || s.verifier.is_some()),
        "nothing judges the run before it opens a pull request"
    );
}

/// The fix run inherits the review starter's premise failure — for the reason
/// [`the_review_starter_measures_no_baseline_of_its_own`] gives — on one of its
/// two launch paths. Launched with `origin: {kind:'branch'}`, the pull
/// request's branch is the base, and a node at the head of this graph measures
/// it correctly. Launched with `kind:'ref'` — every fork pull request, and every
/// one whose head repository cannot be pushed to — the run is cut at the pull
/// request's head while `resolve_base_sha` resolves the merge-base, so the node
/// records a sha no subtraction ever matches. The lazy fallback measures the
/// right merge-base on the red path of both launch paths, so dropping the node
/// loses nothing on a red run and saves a full gate run on every green one. A
/// node conditional on the launch origin would need engine support that does
/// not exist.
#[test]
fn the_fix_starter_measures_no_baseline_of_its_own() {
    let steps = parse(ADDRESS_REVIEW);
    assert!(
        steps.iter().all(|s| s.measure_baseline != Some(true)),
        "a fix launched from a pull request's head records that head as its \
         baseline, which the merge-base subtraction never matches"
    );
    assert_eq!(
        steps.first().map(|s| s.id.0.as_str()),
        Some("s-address"),
        "the fix run opens on the work, not on a measurement"
    );
}

/// Seeding appends a version whenever the bundle stops matching storage, so a
/// starter whose JSON does not survive the round trip through `StepConfig`
/// mints a new version on every launch, forever.
#[test]
fn the_review_starter_seeds_once_and_then_stands_still() {
    let starter = StarterDefinition::parse(CODE_REVIEW).expect("the shipped starter is JSON");
    assert_eq!(plan_seed(&starter, None, None), SeedAction::Create);

    let workflows = repo();
    super::seed_starter_workflows(&workflows);
    let stored = workflows
        .get(&starter.id)
        .expect("read the seeded workflow")
        .expect("the review starter is seeded");
    let latest = workflows
        .latest_version(&starter.id)
        .expect("read the seeded version");

    assert_eq!(
        plan_seed(&starter, Some(&stored), latest.as_ref()),
        SeedAction::Skip
    );
}

// ── Version history: restore + per-version graph (P3.4) ──────────────────
//
// These drive `super::restore_version` / `super::version_graph` — the cores the
// `#[tauri::command]` wrappers delegate to — against a real SQLite repository,
// so the immutability claim ("restore creates a new version row") is proven
// against the storage that actually holds it rather than a stand-in.

/// An in-memory workflow repository with migrations applied.
fn repo() -> Arc<dyn WorkflowRepository> {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    Arc::new(SqliteAdapter::new(conn).expect("run migrations")) as Arc<dyn WorkflowRepository>
}

/// A minimal but *real* v1 step list — `restore` copies this string verbatim,
/// so the tests assert on the exact bytes.
fn steps_json(step_ids: &[&str]) -> String {
    let steps: Vec<serde_json::Value> = step_ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "kind": "agent",
                "title": format!("Step {id}"),
                "agent_kind": null,
                "prompt_template": format!("do {id}"),
                "on_failure": null,
                "max_iterations": null,
            })
        })
        .collect();
    let json = serde_json::to_string(&steps).expect("serialize steps");
    serde_json::from_str::<Vec<StepConfig>>(&json).expect("fixture deserializes into StepConfig");
    json
}

/// A workflow with one version per entry in `versions`, numbered from 1.
fn seed(workflows: &Arc<dyn WorkflowRepository>, id: &str, versions: &[String]) -> WorkflowId {
    let wf_id = WorkflowId::from(id.to_string());
    workflows
        .create(Workflow {
            id: wf_id.clone(),
            name: "History Test".to_string(),
            description: "seeded".to_string(),
            is_starter: false,
            created_at: 1_000,
            updated_at: 1_000,
            schedule: None,
        })
        .expect("create workflow");
    for (i, steps) in versions.iter().enumerate() {
        let n = i as u32 + 1;
        workflows
            .save_version(WorkflowVersion {
                id: WorkflowVersionId::from(format!("{id}-v{n}")),
                workflow_id: wf_id.clone(),
                version: n,
                steps_json: steps.clone(),
                definition_json: None,
                note: Some(format!("v{n}")),
                created_at: 1_000 + i64::from(n),
            })
            .expect("save version");
    }
    wf_id
}

/// The Done-when: restoring appends, and the row it copied is untouched.
#[test]
fn restore_appends_a_verbatim_copy_and_leaves_history_intact() {
    let workflows = repo();
    let v1_steps = steps_json(&["plan"]);
    let v2_steps = steps_json(&["plan", "implement"]);
    let wf_id = seed(&workflows, "wf-hist", &[v1_steps.clone(), v2_steps.clone()]);

    let restored = super::restore_version(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-hist-v1".to_string()),
    )
    .expect("restore v1");

    assert_eq!(restored.version, 3, "restore mints the next version");
    assert_eq!(restored.version_id, "wf-hist-v3");
    assert_eq!(restored.steps.len(), 1);

    let rows = workflows.versions(&wf_id).expect("list versions");
    assert_eq!(rows.len(), 3, "history grew, nothing was replaced");
    assert_eq!(rows[0].steps_json, v1_steps, "v1 is untouched");
    assert_eq!(rows[1].steps_json, v2_steps, "v2 is untouched");
    assert_eq!(
        rows[2].steps_json, v1_steps,
        "the new version is a byte-exact copy of the restored one"
    );
    assert_eq!(rows[2].note.as_deref(), Some("Restored from v1"));
}

/// Name/description aren't versioned, so a content restore must not rewrite
/// them — the workflow keeps the name it had.
#[test]
fn restore_leaves_workflow_metadata_alone() {
    let workflows = repo();
    let wf_id = seed(&workflows, "wf-meta", &[steps_json(&["plan"])]);
    workflows
        .update_meta(&wf_id, "Renamed Later", "new description")
        .expect("rename");

    let restored = super::restore_version(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-meta-v1".to_string()),
    )
    .expect("restore v1");

    assert_eq!(restored.name, "Renamed Later");
    assert_eq!(restored.description, "new description");
}

/// Version ids are guessable (`<workflow-id>-v3`), so the pairing is checked.
#[test]
fn restore_refuses_a_version_from_another_workflow() {
    let workflows = repo();
    let mine = seed(&workflows, "wf-mine", &[steps_json(&["plan"])]);
    seed(&workflows, "wf-theirs", &[steps_json(&["secret"])]);

    let err = match super::restore_version(
        &workflows,
        &mine,
        &WorkflowVersionId::from("wf-theirs-v1".to_string()),
    ) {
        Ok(_) => panic!("a cross-workflow restore must be refused"),
        Err(e) => e,
    };
    assert!(
        format!("{err:?}").contains("wf-theirs"),
        "error names the mismatch: {err:?}"
    );
    assert_eq!(
        workflows.versions(&mine).expect("list").len(),
        1,
        "and writes nothing"
    );
}

/// The drawer diffs a *named* version, not the latest one.
#[test]
fn version_graph_projects_the_named_version() {
    let workflows = repo();
    let wf_id = seed(
        &workflows,
        "wf-graph",
        &[steps_json(&["plan"]), steps_json(&["plan", "implement"])],
    );

    let older = super::version_graph(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-graph-v1".to_string()),
    )
    .expect("graph for v1");
    let newer = super::version_graph(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-graph-v2".to_string()),
    )
    .expect("graph for v2");

    assert_eq!(older.schema_version, 2);
    assert_eq!(
        older
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["plan"]
    );
    assert!(older.edges.is_empty());
    assert_eq!(
        newer
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["plan", "implement"]
    );
    assert_eq!(newer.edges.len(), 1, "list order became a chain edge");
}

// ── v2 persistence (task P3.6) ───────────────────────────────────────────
//
// The prerequisite P3.3 flagged: the builder produces a schema-v2 graph, and
// four things it holds — node positions, join semantics, per-class retry, and
// edge guards — have no v1 representation. These drive `super::save_definition`
// (the `workflow_save` command's core) against a real SQLite repository, so
// "nothing the author drew is lost on save" is proven against the storage that
// actually holds it.

use crate::domain::models::workflow_v2::WorkflowDefinitionV2;

/// A graph that uses every construct v1 cannot express.
fn v2_graph(id: &str) -> WorkflowDefinitionV2 {
    serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": id,
        "name": "Authored",
        "nodes": [
            {
                "id": "plan", "type": "agent", "title": "Plan",
                "config": { "prompt_template": "plan it" },
                "position": { "x": 12.5, "y": 0.0 }
            },
            {
                "id": "scan", "type": "agent", "title": "Security scan",
                "config": { "prompt_template": "scan it" },
                "position": { "x": 320.0, "y": 160.0 }
            },
            {
                "id": "build", "type": "agent", "title": "Build",
                "config": { "prompt_template": "build it" },
                "position": { "x": 0.0, "y": 160.0 },
                "retry": {
                    "environment": { "strategy": "in_place", "max_attempts": 2, "backoff_secs": 30 }
                }
            },
            {
                "id": "ship", "type": "finalize", "title": "Ship",
                "config": {},
                "position": { "x": 160.0, "y": 320.0 },
                "join": "all_done"
            }
        ],
        "edges": [
            { "from": "plan", "to": "build" },
            { "from": "plan", "to": "scan" },
            { "from": "build", "to": "ship" },
            { "from": "scan", "to": "ship", "when": "${{ nodes.scan.outputs.verdict != 'FAIL' }}" }
        ]
    }))
    .expect("fixture is a valid v2 definition")
}

/// The Done-when of the persistence prerequisite: everything the author drew
/// comes back, byte for byte.
#[test]
fn saving_a_v2_graph_preserves_what_v1_cannot_hold() {
    let workflows = repo();
    let saved = super::save_definition(
        &workflows,
        None,
        "Authored",
        "built in the canvas",
        v2_graph("placeholder"),
        None,
    )
    .expect("save");

    let wf_id = WorkflowId::from(saved.id.clone());
    let version = workflows
        .latest_version(&wf_id)
        .expect("read")
        .expect("a version exists");
    let reloaded = version.definition("Authored");

    let node = |id: &str| reloaded.nodes.iter().find(|n| n.id.0 == id).expect(id);
    assert_eq!(node("scan").position.map(|p| p.x), Some(320.0), "layout");
    assert_eq!(
        node("ship").join,
        Some(crate::domain::models::workflow_v2::JoinSemantics::AllDone)
    );
    assert_eq!(
        node("build")
            .retry
            .as_ref()
            .and_then(|r| r.environment.as_ref())
            .map(|r| r.max_attempts),
        Some(Some(2)),
        "per-class retry"
    );
    assert!(
        reloaded.edges.iter().any(|e| e.when.is_some()),
        "edge guard survived"
    );
    assert_eq!(reloaded.edges.len(), 4, "the fan-out/fan-in shape survived");
}

/// The compatibility half: the v1 projection is written too, so the runner,
/// replay, and every pre-P3.6 reader still see a runnable step list.
#[test]
fn saving_also_writes_a_runnable_v1_projection() {
    let workflows = repo();
    let saved = super::save_definition(
        &workflows,
        None,
        "Authored",
        "",
        v2_graph("placeholder"),
        None,
    )
    .expect("save");

    let version = workflows
        .latest_version(&WorkflowId::from(saved.id))
        .expect("read")
        .expect("a version exists");
    let steps: Vec<StepConfig> =
        serde_json::from_str(&version.steps_json).expect("projection parses as v1");

    let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), 4);
    let at = |id: &str| ids.iter().position(|s| *s == id).expect("present");
    assert!(at("plan") < at("build") && at("build") < at("ship"));
    assert!(version.definition_json.is_some(), "and the v2 document too");
}

/// The workflow's own identity wins over whatever the definition claims —
/// otherwise a graph copied from another workflow (or a template's placeholder
/// id) would travel into storage and the version would describe the wrong one.
#[test]
fn the_stored_definition_is_normalized_to_the_workflow_it_belongs_to() {
    let workflows = repo();
    let saved = super::save_definition(
        &workflows,
        None,
        "Renamed",
        "",
        v2_graph("wf-some-other-workflow"),
        None,
    )
    .expect("save");

    let version = workflows
        .latest_version(&WorkflowId::from(saved.id.clone()))
        .expect("read")
        .expect("exists");
    let reloaded = version.definition("Renamed");
    assert_eq!(reloaded.id.0, saved.id);
    assert_eq!(reloaded.name, "Renamed");
}

/// Editing appends; it never rewrites. Same guarantee the restore path has.
#[test]
fn saving_an_existing_workflow_appends_a_version() {
    let workflows = repo();
    let first = super::save_definition(&workflows, None, "W", "", v2_graph("x"), None).expect("v1");
    assert_eq!(first.version, 1);

    let mut edited = v2_graph("x");
    edited.nodes.retain(|n| n.id.0 != "scan");
    edited
        .edges
        .retain(|e| e.from.0 != "scan" && e.to.0 != "scan");
    let second = super::save_definition(
        &workflows,
        Some(WorkflowId::from(first.id.clone())),
        "W",
        "",
        edited,
        Some("dropped the scan branch".to_string()),
    )
    .expect("v2");

    assert_eq!(second.version, 2);
    let rows = workflows
        .versions(&WorkflowId::from(first.id))
        .expect("list");
    assert_eq!(rows.len(), 2, "history grew");
    assert_eq!(
        rows[0].definition_json.as_ref().map(|d| d.contains("scan")),
        Some(true),
        "v1 still holds the branch it was saved with"
    );
}

/// P3.3's guarantee, at the write path rather than the Save button: a graph
/// with an error-severity finding cannot be stored at all.
#[test]
fn a_structurally_invalid_graph_is_refused_by_the_write_path() {
    let workflows = repo();
    let mut broken = v2_graph("x");
    // Two finalize sinks — the `multiple-finalize` lint error.
    broken.nodes.push(
        serde_json::from_value(serde_json::json!({
            "id": "ship2", "type": "finalize", "title": "Ship again", "config": {}
        }))
        .unwrap(),
    );
    broken.edges.push(
        serde_json::from_value(serde_json::json!({ "from": "build", "to": "ship2" })).unwrap(),
    );

    let err = match super::save_definition(&workflows, None, "W", "", broken, None) {
        Err(e) => e,
        Ok(_) => panic!("a graph with a structural error must not be storable"),
    };
    assert!(
        format!("{err:?}").contains("structural errors"),
        "unexpected error: {err:?}"
    );
}
