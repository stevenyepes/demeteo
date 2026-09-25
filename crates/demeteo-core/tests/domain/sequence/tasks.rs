// Tests extracted from `crates/demeteo-core/src/domain/sequence/tasks.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn task(id: &str, title: &str, files: &[&str]) -> PlannedTask {
    PlannedTask {
        id: id.into(),
        title: title.into(),
        description: format!("d-{id}"),
        files: files.iter().map(|f| (*f).into()).collect(),
        test_command: None,
        acceptance: vec![],
        blocked_by: vec![],
        retry_note: None,
    }
}

fn plan() -> TaskPlan {
    TaskPlan {
        tasks: vec![
            task(
                "task-1",
                "backend",
                &["src/api/mod.rs", "src/api/routes.rs"],
            ),
            task("task-2", "frontend", &["ui/App.tsx"]),
        ],
        already_landed: vec![],
        resumes_landed_work: false,
        ..Default::default()
    }
}

#[test]
fn selects_only_tasks_owning_implicated_files() {
    let out = select_targeted_tasks(&plan(), "fix the route", &["src/api/routes.rs".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "task-1");
    assert_eq!(out.tasks[0].retry_note.as_deref(), Some("fix the route"));
}

/// The tasks a targeted retry skips are still in the worktree it opens — the
/// branch carries them from the previous attempt. If they don't come back in
/// `already_landed`, the running task's prompt claims an empty branch and the
/// agent reimplements work it is sitting on.
#[test]
fn skipped_tasks_are_reported_as_already_landed() {
    let out = select_targeted_tasks(&plan(), "fix the route", &["src/api/routes.rs".into()]);
    assert_eq!(out.already_landed.len(), 1);
    assert_eq!(out.already_landed[0].id, "task-2");
    assert!(out.resumes_landed_work);
}

#[test]
fn empty_implicated_files_falls_back_to_all_tasks() {
    let out = select_targeted_tasks(&plan(), "fb", &[]);
    assert_eq!(out.tasks.len(), 2);
    assert!(out
        .tasks
        .iter()
        .all(|t| t.retry_note.as_deref() == Some("fb")));
    // Nothing is skipped, so nothing is "already landed" — but the branch
    // still holds the previous attempt, which is what this flag says.
    assert!(out.already_landed.is_empty());
    assert!(out.resumes_landed_work);
}

#[test]
fn unmatched_implicated_files_fall_back_to_all_tasks() {
    let out = select_targeted_tasks(&plan(), "fb", &["totally/else.rs".into()]);
    assert_eq!(out.tasks.len(), 2);
}

#[test]
fn dot_slash_prefix_is_normalized() {
    let out = select_targeted_tasks(&plan(), "fb", &["./ui/App.tsx".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "task-2");
    assert_eq!(out.already_landed.len(), 1);
    assert_eq!(out.already_landed[0].id, "task-1");
}

/// A plan read off the task-list artifact is a fresh, full plan — it must not
/// come back claiming the branch already holds an implementation.
#[test]
fn a_parsed_plan_does_not_claim_landed_work() {
    let body = r#"{"tasks":[{"id":"task-1","title":"t","description":"d","files":["a.rs"]}]}"#;
    let plan = extract_task_plan(body).expect("bare JSON should parse");
    assert!(!plan.resumes_landed_work);
    assert!(plan.already_landed.is_empty());
}

/// The task-list artifact is written by the spec agent as a plain JSON file,
/// so the common case is bare JSON with no fence and no prose around it.
#[test]
fn parses_a_bare_json_task_list_artifact() {
    let body = r#"{"tasks":[{"id":"task-1","title":"t","description":"d","files":["a.rs"]}]}"#;
    let plan = extract_task_plan(body).expect("bare JSON should parse");
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].id, "task-1");
}

/// Agents wrap JSON in a fence even when told to write a .json file, and the
/// legacy planner path streams a whole turn. Both must still yield a plan.
#[test]
fn parses_a_fenced_task_list_with_surrounding_prose() {
    let body = "Here is the plan:\n\n```json\n{\"tasks\":[{\"id\":\"task-1\",\"title\":\"t\",\"description\":\"d\"}]}\n```\n\nLet me know.";
    let plan = extract_task_plan(body).expect("fenced JSON should parse");
    assert_eq!(plan.tasks.len(), 1);
}

/// A workflow still on the old `parallel` kind runs through the legacy planner
/// fallback, whose prompt asks for `subtasks`. Dispatching it to the sequence
/// handler must not break it, so the alias has to deserialize.
#[test]
fn parses_the_legacy_subtasks_key() {
    let body = r#"{"subtasks":[{"id":"sub-1","title":"t","description":"d","files":["a.rs"]}]}"#;
    let plan = extract_task_plan(body).expect("legacy `subtasks` key should parse");
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].id, "sub-1");
}

#[test]
fn parses_a_generic_fence_with_no_language_tag() {
    let text = "```\n{\"tasks\": [{\"id\": \"t\", \"title\": \"T\", \"description\": \"D\", \"files\": []}]}\n```";
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks[0].id, "t");
}

#[test]
fn parses_a_bare_object_embedded_in_prose() {
    let text = r#"The plan is: {"tasks": [{"id": "x", "title": "T", "description": "D", "files": []}]} and that's it."#;
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks[0].id, "x");
}

/// The balanced-brace scan must not be fooled by braces inside string values,
/// or it truncates the object and the whole plan fails to parse.
#[test]
fn parses_nested_braces_inside_strings() {
    let text = r#"```json
{"tasks": [{"id": "a", "title": "{nested}", "description": "}", "files": []}]}
```"#;
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks[0].title, "{nested}");
}

#[test]
fn parses_multiple_tasks_preserving_order() {
    let text = r#"```json
{"tasks": [
  {"id": "a", "title": "A", "description": "do A", "files": ["x.rs"]},
  {"id": "b", "title": "B", "description": "do B", "files": ["y.rs"]}
]}
```"#;
    let plan = extract_task_plan(text).expect("should parse");
    assert_eq!(plan.tasks.len(), 2);
    // Order is the contract: task b runs against a worktree containing a.
    assert_eq!(plan.tasks[0].id, "a");
    assert_eq!(plan.tasks[1].id, "b");
}

#[test]
fn rejects_text_with_no_task_list() {
    assert!(extract_task_plan("I could not decompose this.").is_none());
}

/// The turn this was reported from: the prose named `{feature_id}` and a
/// shell variable before the plan, so the first balanced object in the text
/// was an identifier in braces. A scan that evaluated one candidate stopped
/// there and failed the step with "could not read a task list", over a list
/// sitting two lines below.
#[test]
fn braced_identifiers_in_prose_do_not_hide_the_task_list() {
    let text = "Each session is keyed `{feature_id}-{step_id}` and the harness runs `${TEST_CMD}`.\n\
                {\"tasks\": [{\"id\": \"a\", \"title\": \"T\", \"description\": \"D\", \"files\": []}]}";
    let plan = extract_task_plan(text).expect("the object below the prose is the plan");
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].id, "a");
}

/// The planner prompt quotes a *filled-in* example of the shape and then asks
/// for the real one, so a turn that echoes it before answering carries two
/// objects that both deserialize. Preferring the earlier one is not a parse
/// failure here — it is a run of one task whose description is `...`.
#[test]
fn an_echoed_shape_example_does_not_win_over_the_real_plan() {
    let example = task_list_json_shape_example(false);
    let real = r#"{"tasks": [{"id": "real-1", "title": "T", "description": "D", "files": []}]}"#;
    let fenced =
        format!("As asked:\n\n```json\n{example}\n```\n\nThe plan:\n\n```json\n{real}\n```");
    let bare = format!("As asked: {example}. And the plan itself: {real}");
    for text in [&fenced, &bare] {
        let plan = extract_task_plan(text).expect("a plan is there");
        assert_eq!(plan.tasks.len(), 1, "{text}");
        assert_eq!(plan.tasks[0].id, "real-1", "{text}");
    }
}

/// Every candidate that deserializes is accepted, so the only thing keeping an
/// unrelated object from answering as a plan with no tickets is that `tasks`
/// carries no `#[serde(default)]`. The caller reports "no tickets" and "could
/// not read a task list" completely differently, so this is the guard.
#[test]
fn an_object_that_is_not_a_task_list_is_unreadable_rather_than_empty() {
    assert!(extract_task_plan(r#"Nothing doing. {"status": "ok", "tickets": []}"#).is_none());
}

/// The other side of that guard: a rework producer that deliberately wrote no
/// tickets must still parse when its object is not the only one in the text,
/// or a sanctioned "nothing here a ticket can fix" degrades into a read
/// failure and the producer's reason never reaches the human.
#[test]
fn an_empty_list_still_parses_when_prose_precedes_it() {
    let text = "Nothing an implementation ticket can close (`{scope}` is unrelated).\n\
                {\"kind\": \"rework\", \"tasks\": [], \"notes\": \"the gap is in project config\"}";
    let plan = extract_task_plan(text).expect("an empty list is a plan, not a read failure");
    assert!(plan.tasks.is_empty());
    assert_eq!(plan.kind, PlanKind::Rework);
    assert_eq!(plan.notes.as_deref(), Some("the gap is in project config"));
}

fn plan_of(ids: &[&str]) -> TaskPlan {
    TaskPlan {
        tasks: ids.iter().map(|id| task(id, "t", &[])).collect(),
        already_landed: vec![],
        resumes_landed_work: false,
        ..Default::default()
    }
}

// --- apply_landed_checkpoint -------------------------------------------------
//
// After a mid-list failure, the completed prefix is merged to the feature
// branch and its task ids recorded as a checkpoint. Every subsequent plan for
// that step runs through this filter, so a retry pays only for the remainder.

#[test]
fn checkpointed_tasks_are_skipped_and_reported_as_landed() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b", "c"]), &["a".into(), "b".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "c");
    // The skipped tasks must still be named in the running task's prompt —
    // the worktree it opens contains their merged work.
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["a", "b"]);
    assert!(out.resumes_landed_work);
}

#[test]
fn a_plan_with_no_checkpointed_tasks_is_untouched() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b"]), &["z".into()]);
    assert_eq!(out.tasks.len(), 2);
    assert!(out.already_landed.is_empty());
    assert!(!out.resumes_landed_work);
}

/// A checkpoint covering every task means every task is done — the state a
/// kill between the last task's commit and the step's merge leaves behind.
///
/// This used to put the whole plan back, on the premise that all-ids-matched
/// could only mean a stale row. Under V35 it is the *fresh* state, and
/// re-running it is the exact cost the checkpoint exists to avoid: a 25-task
/// step killed during the verifier would re-pay for all 25. The caller has
/// already verified the anchor against the repo, so the work is there.
#[test]
fn a_checkpoint_covering_every_task_leaves_nothing_to_run() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b"]), &["a".into(), "b".into()]);
    assert!(
        out.tasks.is_empty(),
        "every task landed, so none may re-run; got {:?}",
        out.tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    // Still named, so the step's tail can tell "resumed, nothing to do" from
    // "the task list was empty", which is a misconfiguration.
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["a", "b"]);
    assert!(out.resumes_landed_work);
}

/// Checkpoint filtering composes with a targeted retry's own skip-list: the
/// landed tasks join `already_landed` rather than replacing what selection
/// already put there.
#[test]
fn checkpoint_extends_an_existing_already_landed_list() {
    let mut plan = plan_of(&["b", "c"]);
    plan.already_landed = plan_of(&["a"]).tasks;
    plan.resumes_landed_work = true;
    let out = apply_landed_checkpoint(plan, &["b".into()]);
    assert_eq!(out.tasks.len(), 1);
    assert_eq!(out.tasks[0].id, "c");
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["a", "b"]);
}

/// Order is the sequence step's contract; the filter must not reshuffle the
/// tasks that remain.
#[test]
fn checkpoint_filter_preserves_task_order() {
    let out = apply_landed_checkpoint(plan_of(&["a", "b", "c", "d"]), &["b".into()]);
    let ids: Vec<&str> = out.tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["a", "c", "d"]);
}

#[test]
fn a_well_formed_plan_validates() {
    assert!(validate_task_plan(&plan_of(&["task-1", "task-2"])).is_none());
}

/// Task ids key the agent session (`{feature}-{step}-{task}`) and the
/// completed-task record. Two tasks sharing one id collide on both, and
/// `select_targeted_tasks` would collapse them into a single entry.
#[test]
fn rejects_duplicate_task_ids() {
    let reason = validate_task_plan(&plan_of(&["task-1", "task-1"])).expect("should reject");
    assert!(reason.contains("task-1"), "{reason}");
}

#[test]
fn rejects_an_empty_task_id() {
    let reason = validate_task_plan(&plan_of(&["task-1", "   "])).expect("should reject");
    assert!(reason.contains("empty `id`"), "{reason}");
}

/// There is no cap on task count: decomposition is sized by the ticket
/// rubric, and cost is bounded per task (budget ceiling) rather than per
/// list. A long, well-formed list must validate.
#[test]
fn a_long_task_list_validates() {
    let ids: Vec<String> = (0..60).map(|i| format!("task-{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    assert!(validate_task_plan(&plan_of(&refs)).is_none());
}

// --- blocked_by --------------------------------------------------------------
//
// Tasks run strictly in list order; `blocked_by` edges are validated against
// that order and drive the targeted-retry closure below.

#[test]
fn rejects_a_forward_blocked_by_edge() {
    let mut plan = plan_of(&["a", "b"]);
    plan.tasks[0].blocked_by = vec!["b".into()];
    let reason = validate_task_plan(&plan).expect("should reject");
    assert!(reason.contains("not an earlier task"), "{reason}");
}

#[test]
fn rejects_a_blocked_by_edge_to_a_missing_task() {
    let mut plan = plan_of(&["a", "b"]);
    plan.tasks[1].blocked_by = vec!["ghost".into()];
    let reason = validate_task_plan(&plan).expect("should reject");
    assert!(reason.contains("ghost"), "{reason}");
}

#[test]
fn rejects_a_self_blocked_task() {
    let mut plan = plan_of(&["a"]);
    plan.tasks[0].blocked_by = vec!["a".into()];
    let reason = validate_task_plan(&plan).expect("should reject");
    assert!(reason.contains("itself"), "{reason}");
}

#[test]
fn accepts_backward_blocked_by_edges() {
    let mut plan = plan_of(&["a", "b", "c"]);
    plan.tasks[2].blocked_by = vec!["a".into(), "b".into()];
    assert!(validate_task_plan(&plan).is_none());
}

/// Re-running a foundation task rewrites what its dependents were built on,
/// so a targeted retry must pull them in transitively — otherwise they are
/// reported as landed work that still matches a branch that just changed
/// under them.
#[test]
fn targeted_retry_reruns_transitive_dependents_of_a_selected_task() {
    let mut plan = plan_of(&["base", "mid", "leaf", "unrelated"]);
    plan.tasks[0].files = vec!["src/base.rs".into()];
    plan.tasks[1].blocked_by = vec!["base".into()];
    plan.tasks[2].blocked_by = vec!["mid".into()];
    plan.tasks[3].files = vec!["src/other.rs".into()];

    let out = select_targeted_tasks(&plan, "fix base", &["src/base.rs".into()]);
    let ids: Vec<&str> = out.tasks.iter().map(|t| t.id.as_str()).collect();
    // List order is preserved, and only the untouched task is skipped.
    assert_eq!(ids, ["base", "mid", "leaf"]);
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["unrelated"]);
}

/// `blocked_by` is a planner-declared edge and can be incomplete. A task
/// that shares a file with a task the retry is already re-running is
/// re-run too, even with no `blocked_by` edge between them at all —
/// otherwise an omitted dependency silently ships stale code built on a
/// foundation that just changed underneath it.
#[test]
fn targeted_retry_reruns_a_task_that_shares_a_file_with_a_selected_task_even_without_a_blocked_by_edge(
) {
    let mut plan = plan_of(&["base", "mid", "leaf", "unrelated"]);
    plan.tasks[0].files = vec!["src/base.rs".into()];
    // `mid` is pulled in via a declared `blocked_by` edge, not by owning an
    // implicated file directly.
    plan.tasks[1].blocked_by = vec!["base".into()];
    plan.tasks[1].files = vec!["src/mid.rs".into()];
    // `leaf` declares NO `blocked_by` edge to `mid` — the omission this test
    // exists for — but shares `src/mid.rs` with it.
    plan.tasks[2].files = vec!["src/mid.rs".into(), "src/leaf.rs".into()];
    plan.tasks[3].files = vec!["src/other.rs".into()];

    let out = select_targeted_tasks(&plan, "fix base", &["src/base.rs".into()]);
    let ids: Vec<&str> = out.tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["base", "mid", "leaf"]);
    let landed: Vec<&str> = out.already_landed.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(landed, ["unrelated"]);
}

// --- is_rework_plan ----------------------------------------------------------
//
// Whether a task list is a delta against work already on the branch or a
// fresh whole decomposition. The producing step declares it; the id-overlap
// fallback exists because that declaration is written by an agent following
// a prompt, not enforced by a schema.

#[test]
fn a_declared_rework_plan_is_a_delta_whatever_its_ids() {
    // The declaration wins outright — including the awkward case where a
    // producer reused an id from the original decomposition because the
    // rework genuinely revisits that ticket.
    let mut incoming = plan_of(&["a", "fix-1"]);
    incoming.kind = PlanKind::Rework;
    let previous = plan_of(&["a", "b"]);
    assert!(is_rework_plan(&incoming, Some(&previous)));
}

#[test]
fn an_undeclared_list_sharing_no_ids_is_read_as_a_delta() {
    // The fallback: a producer that emitted a delta but forgot the marker.
    // Read as greenfield, its agents would be told the branch is empty
    // while standing in a worktree holding the whole previous cycle.
    let incoming = plan_of(&["fix-1", "fix-2"]);
    let previous = plan_of(&["a", "b"]);
    assert!(is_rework_plan(&incoming, Some(&previous)));
}

#[test]
fn an_undeclared_list_reusing_any_id_is_a_revised_whole_list() {
    // A gate saying "the split is too coarse" gets back a re-decomposition
    // that keeps most ids. One shared id is enough to say so: a delta names
    // work that did not exist before.
    let incoming = plan_of(&["a", "b", "c"]);
    let previous = plan_of(&["a", "b"]);
    assert!(!is_rework_plan(&incoming, Some(&previous)));
}

#[test]
fn the_first_cycle_is_never_a_delta() {
    // Nothing precedes it, so there is nothing for a delta to be against —
    // and an empty previous cycle trivially shares no ids, which is exactly
    // the shape the overlap test would otherwise misread.
    let incoming = plan_of(&["a", "b"]);
    assert!(!is_rework_plan(&incoming, None));
    assert!(!is_rework_plan(&incoming, Some(&plan_of(&[]))));
}

#[test]
fn an_empty_incoming_list_is_not_a_delta() {
    // Vacuously shares no ids with anything. Calling it a delta would mean
    // "zero tasks close the verdict", which the step would then run as a
    // no-op cycle instead of failing on an empty list.
    let incoming = plan_of(&[]);
    let previous = plan_of(&["a"]);
    assert!(!is_rework_plan(&incoming, Some(&previous)));
}

#[test]
fn ids_are_compared_trimmed() {
    // `validate_task_plan` accepts a whitespace-padded id, so an untrimmed
    // comparison would see " a " and "a" as different work and call a
    // revision a delta.
    let mut incoming = plan_of(&[" a "]);
    incoming.tasks[0].id = " a ".to_string();
    let previous = plan_of(&["a"]);
    assert!(!is_rework_plan(&incoming, Some(&previous)));
}

// --- cycle history -----------------------------------------------------------

#[test]
fn closing_a_cycle_stacks_it_onto_the_history() {
    let first = plan_of(&["a", "b"]);
    let mut second = plan_of(&["fix-1"]);
    second.kind = PlanKind::Rework;
    second.cycle = 1;
    second.history = first.close_cycle();

    assert_eq!(second.history.len(), 1);
    assert_eq!(second.history[0].cycle, 0);
    assert_eq!(second.history[0].kind, PlanKind::Greenfield);

    // A third cycle carries both earlier ones, oldest first.
    let mut third = plan_of(&["fix-2"]);
    third.cycle = 2;
    third.history = second.close_cycle();
    let cycles: Vec<u32> = third.history.iter().map(|c| c.cycle).collect();
    assert_eq!(cycles, [0, 1]);

    // And every task from both is what its agents are told is on the branch.
    let prior: Vec<String> = third.all_prior_tasks().into_iter().map(|t| t.id).collect();
    assert_eq!(prior, ["a", "b", "fix-1"]);
}

#[test]
fn a_greenfield_plans_history_serializes_away_entirely() {
    // The back-compat claim for the plan cache: an untouched row's JSON is
    // byte-identical to what it was before cycles existed, so an older
    // build reads it unchanged.
    let json = serde_json::to_string(&plan_of(&["a"])).expect("serializes");
    assert!(!json.contains("history"), "{json}");
    assert!(json.contains("\"kind\":\"greenfield\""), "{json}");
}

#[test]
fn a_pre_cycle_plan_json_still_parses() {
    let legacy = r#"{"tasks":[{"id":"a","title":"t","description":"d"}]}"#;
    let plan: TaskPlan = serde_json::from_str(legacy).expect("legacy row parses");
    assert_eq!(plan.kind, PlanKind::Greenfield);
    assert_eq!(plan.cycle, 0);
    assert!(plan.history.is_empty());
}

#[test]
fn the_overlap_fallback_sees_the_cached_plans_own_tasks_not_only_its_history() {
    // The first rework cycle is the one that matters most, and it is the one
    // where `history` is still empty: the cached plan holds the original 25
    // tickets in `tasks`, with nothing behind it. Comparing against history
    // alone would make `previous` empty, so an undeclared delta would read
    // as greenfield and every ticket would re-run — exactly the bug this
    // whole change exists to remove, reintroduced in the fallback.
    let greenfield = plan_of(&["ticket-01", "ticket-02"]);
    assert!(greenfield.history.is_empty());

    let incoming = plan_of(&["fix-1"]); // undeclared delta
    assert!(
        is_rework_plan(&incoming, Some(&greenfield)),
        "a delta sharing no id with the cached plan's own tasks is a delta"
    );

    let revision = plan_of(&["ticket-01", "ticket-02", "ticket-03"]);
    assert!(
        !is_rework_plan(&revision, Some(&greenfield)),
        "a re-decomposition reusing ids is not"
    );
}

/// The rework prompt tells the producer to write no tickets when the review
/// named nothing a ticket can fix, and to say why. `notes` was not a field,
/// so serde dropped that "why" on the floor and the run failed with a
/// message about unreadable JSON — over an artifact that parsed perfectly.
#[test]
fn a_producers_reason_for_an_empty_rework_list_survives_the_parse() {
    let body = r#"{"kind": "rework", "tasks": [],
        "notes": "the failing check is a project-configuration gap, not an implementation defect"}"#;
    let plan = extract_task_plan(body).expect("an empty rework list is still valid JSON");
    assert_eq!(plan.kind, PlanKind::Rework);
    assert!(plan.tasks.is_empty());
    assert_eq!(
        plan.notes.as_deref(),
        Some("the failing check is a project-configuration gap, not an implementation defect")
    );
}

/// `notes` is the producer's contract, not execution state, so a plan that
/// carries none must serialize exactly as it did before the field existed —
/// the durable plan cache stores this JSON.
#[test]
fn a_plan_without_notes_serializes_without_the_key() {
    let json = serde_json::to_string(&plan()).expect("serializes");
    assert!(!json.contains("notes"), "{json}");
}

#[test]
fn a_short_task_title_becomes_the_commit_subject_unchanged() {
    let msg = task_commit_message("f-1d0209a0e43d5b67", "ticket-1", "Add the settings context");
    assert_eq!(msg, "feat(f-1d0209a0e43d5b67): add the settings context");
    assert!(!msg.contains('\n'), "no body when nothing was dropped");
}

/// The loop this exists to break: an agent-written ticket title goes into a
/// commit subject verbatim, the subject busts commitlint's 72, the target
/// repo's own `npm run checks` fails on `origin/master..HEAD`, validate
/// rejects the feature, and the rework ticket raised to fix it contributes
/// its own over-long title as the next bad commit.
#[test]
fn an_over_long_title_is_cut_to_fit_commitlints_limits() {
    let title = "collapse the merged duplicate branch line so no non-compliant commit remains in origin/master..HEAD";
    let msg = task_commit_message("f-1d0209a0e43d5b67", "rework-2", title);
    let header = msg.lines().next().expect("a header");

    let subject = header.split_once(": ").expect("conventional header").1;
    assert!(
        subject.chars().count() <= 72,
        "subject was {} chars: {subject}",
        subject.chars().count()
    );
    assert!(
        header.chars().count() <= 100,
        "header was {} chars",
        header.chars().count()
    );
    assert!(
        !subject.ends_with('.'),
        "commitlint's subject-full-stop rejects it: {subject}"
    );
    assert!(header.starts_with("feat(f-1d0209a0e43d5b67): collapse the merged duplicate"));

    // Nothing is lost — the full title is preserved in the body, wrapped
    // so it cannot trip body-max-line-length in turn.
    let body = msg.split_once("\n\n").expect("a body").1;
    assert_eq!(
        body.split_whitespace().collect::<Vec<_>>().join(" "),
        title.to_lowercase()
    );
    assert!(
        body.lines().all(|l| l.chars().count() <= 100),
        "body line over 100: {body}"
    );
}

/// `subject-case` rejects sentence/start/pascal/upper case, and a title
/// arriving as a sentence is the common shape.
#[test]
fn a_title_is_lowercased_and_stripped_of_its_trailing_period() {
    let msg = task_commit_message("f-abc", "t-1", "Wire The Context Provider.");
    assert_eq!(msg, "feat(f-abc): wire the context provider");
}

/// A title is agent-written and may be blank or whitespace; a header of
/// `feat(f-abc): ` is rejected by `subject-empty` and would fail the commit
/// rather than the lint.
#[test]
fn a_blank_title_falls_back_to_the_task_id() {
    let msg = task_commit_message("f-abc", "ticket-7", "   ");
    assert_eq!(msg, "feat(f-abc): ticket-7");
}

/// A single unbroken token longer than the budget has no word boundary to
/// cut on, and returning an empty subject there would be worse than a
/// mid-token cut.
#[test]
fn a_title_with_no_usable_word_boundary_is_still_cut_to_fit() {
    let title = "a".repeat(200);
    let msg = task_commit_message("f-abc", "t-1", &title);
    let header = msg.lines().next().expect("a header");
    let subject = header.split_once(": ").expect("conventional header").1;
    assert_eq!(subject.chars().count(), 72);
}

// --- who repairs a rejected list --------------------------------------------
//
// The check ("is this executable") and the routing ("whose defect is it")
// are separate questions, and only the second decides whether the run
// survives. These pin the second.

#[test]
fn a_producer_backed_step_sends_a_malformed_list_back_to_its_producer() {
    let mut plan = plan_of(&["a", "b"]);
    plan.tasks[1].blocked_by = vec!["ghost".into()];
    let producer = crate::domain::ids::StepId::from("s-tickets");

    match reject_unexecutable_plan(&plan, Some(&producer)) {
        Some(PlanRejection::ProducerMustFix { producer, reason }) => {
            assert_eq!(producer.0, "s-tickets");
            // The producer is being asked to rewrite the list, so the
            // message has to name the defect, not just report a rejection.
            assert!(reason.contains("ghost"), "{reason}");
            assert!(reason.contains("not an earlier task"), "{reason}");
        }
        other => panic!("expected ProducerMustFix, got {other:?}"),
    }
}

/// A planner-sourced step wrote the list inside itself, so there is no
/// other node to send it to — re-running the step re-runs the planner that
/// just failed.
#[test]
fn a_planner_sourced_step_has_nobody_to_send_it_to() {
    let mut plan = plan_of(&["a", "b"]);
    plan.tasks[1].blocked_by = vec!["ghost".into()];

    match reject_unexecutable_plan(&plan, None) {
        Some(PlanRejection::Terminal { reason }) => {
            assert!(reason.contains("ghost"), "{reason}");
        }
        other => panic!("expected Terminal, got {other:?}"),
    }
}

/// Every rule the check enforces routes, not just the one that bit in
/// production — a list rejected for a duplicate id is as repairable as one
/// rejected for a dangling edge.
#[test]
fn every_rejection_rule_routes_to_the_producer() {
    let producer = crate::domain::ids::StepId::from("s-tickets");

    let duplicate = plan_of(&["a", "a"]);
    let blank = plan_of(&["a", "  "]);
    let mut self_dep = plan_of(&["a"]);
    self_dep.tasks[0].blocked_by = vec!["a".into()];
    let mut forward = plan_of(&["a", "b"]);
    forward.tasks[0].blocked_by = vec!["b".into()];

    for (label, plan) in [
        ("duplicate id", duplicate),
        ("blank id", blank),
        ("self dependency", self_dep),
        ("forward edge", forward),
    ] {
        assert!(
            matches!(
                reject_unexecutable_plan(&plan, Some(&producer)),
                Some(PlanRejection::ProducerMustFix { .. })
            ),
            "{label} should route to the producer"
        );
    }
}

#[test]
fn an_executable_list_is_not_rejected_at_all() {
    let producer = crate::domain::ids::StepId::from("s-tickets");
    assert!(reject_unexecutable_plan(&plan_of(&["a", "b"]), Some(&producer)).is_none());
}

// ---------- reject_stale_rework_plan ----------
//
// A different question again: not "is this list executable" but "is this
// list the delta a rework cycle demands". A producer that opted into
// `rework_prompt_template` and still hands back a whole decomposition under
// rework conditions is a defect — it either wasn't actually re-run before
// this step was re-entered, or ignored its own rework instructions — and
// the fix always lives with the producer, never with re-running it here.

#[test]
fn a_stale_whole_plan_under_rework_is_sent_back_to_the_producer() {
    let producer = crate::domain::ids::StepId::from("s-tickets");
    match reject_stale_rework_plan(true, false, Some(&producer), true, false) {
        Some(PlanRejection::ProducerMustFix { producer, reason }) => {
            assert_eq!(producer.0, "s-tickets");
            assert!(reason.contains("delta"), "{reason}");
        }
        other => panic!("expected ProducerMustFix, got {other:?}"),
    }
}

#[test]
fn a_healthy_delta_is_not_rejected() {
    let producer = crate::domain::ids::StepId::from("s-tickets");
    assert!(reject_stale_rework_plan(true, true, Some(&producer), true, false).is_none());
}

#[test]
fn outside_a_rework_cycle_nothing_is_rejected() {
    // A gate revision re-reading a fresh, whole list is supposed to look
    // exactly like this — greenfield, not a delta — and must not be faulted.
    let producer = crate::domain::ids::StepId::from("s-tickets");
    assert!(reject_stale_rework_plan(false, false, Some(&producer), true, false).is_none());
}

#[test]
fn a_planner_sourced_step_is_never_faulted() {
    assert!(reject_stale_rework_plan(true, false, None, false, false).is_none());
}

#[test]
fn a_producer_with_no_rework_template_keeps_the_old_accepted_cost() {
    // The opt-in gate this shares with the redirect controller's own
    // producer hop: without a `rework_prompt_template` the producer has no
    // way to answer with a delta, so a whole re-decomposition here is the
    // accepted cost, not a defect to send back.
    let producer = crate::domain::ids::StepId::from("s-tickets");
    assert!(reject_stale_rework_plan(true, false, Some(&producer), false, false).is_none());
}

#[test]
fn a_replayed_list_is_sent_back_naming_the_replay() {
    let producer = crate::domain::ids::StepId::from("s-tickets");
    match reject_stale_rework_plan(true, false, Some(&producer), true, true) {
        Some(PlanRejection::ProducerMustFix { reason, .. }) => {
            assert!(reason.contains("already ran last cycle"), "{reason}");
            assert!(!reason.contains("whole decomposition"), "{reason}");
        }
        other => panic!("expected ProducerMustFix, got {other:?}"),
    }
}

// ---------- replays_a_judged_cycle ----------
//
// `kind: rework` survives on disk after the cycle that wrote it, so a
// consumer re-entered without its producer re-running reads last cycle's
// delta as this cycle's. The declaration cannot tell them apart; the
// tickets can.

/// The cache after one rework cycle: `g0` was the greenfield list, `d1`
/// the delta that ran on top of it.
fn cache_after_one_rework() -> TaskPlan {
    let g0 = plan_of(&["a", "b", "c"]);
    let mut d1 = plan_of(&["fix-1", "fix-2"]);
    d1.kind = PlanKind::Rework;
    d1.cycle = 1;
    d1.history = g0.close_cycle();
    d1
}

fn rework_list(ids: &[&str]) -> TaskPlan {
    let mut p = plan_of(ids);
    p.kind = PlanKind::Rework;
    p
}

#[test]
fn a_list_identical_to_a_judged_cycle_is_a_replay() {
    let cached = cache_after_one_rework();
    let incoming = rework_list(&["fix-2", "fix-1"]);
    assert!(is_rework_plan(&incoming, Some(&cached)));
    assert!(replays_a_judged_cycle(&incoming, Some(&cached), true));

    let greenfield_again = plan_of(&["a", "b", "c"]);
    assert!(replays_a_judged_cycle(
        &greenfield_again,
        Some(&cached),
        true
    ));
}

#[test]
fn a_retry_note_does_not_make_a_list_new() {
    let cached = cache_after_one_rework();
    let mut incoming = rework_list(&["fix-1", "fix-2"]);
    incoming.tasks[0].retry_note = Some("from the verdict".into());
    assert!(replays_a_judged_cycle(&incoming, Some(&cached), true));
}

#[test]
fn an_unjudged_cycle_is_never_a_replay() {
    let cached = cache_after_one_rework();
    let incoming = rework_list(&["fix-1", "fix-2"]);
    assert!(!replays_a_judged_cycle(&incoming, Some(&cached), false));
    assert!(!replays_a_judged_cycle(&incoming, None, true));
    assert!(!replays_a_judged_cycle(
        &rework_list(&[]),
        Some(&cached),
        true
    ));
}

#[test]
fn a_revisited_ticket_with_a_new_body_is_not_a_replay() {
    let cached = cache_after_one_rework();

    let mut rewritten = rework_list(&["fix-1", "fix-2"]);
    rewritten.tasks[1].description = "close the empty-state finding".into();
    assert!(!replays_a_judged_cycle(&rewritten, Some(&cached), true));

    let mut new_acceptance = rework_list(&["fix-1", "fix-2"]);
    new_acceptance.tasks[0].acceptance = vec!["renders the empty state".into()];
    assert!(!replays_a_judged_cycle(
        &new_acceptance,
        Some(&cached),
        true
    ));

    let subset = rework_list(&["fix-1"]);
    assert!(!replays_a_judged_cycle(&subset, Some(&cached), true));

    let superset = rework_list(&["fix-1", "fix-2", "fix-3"]);
    assert!(!replays_a_judged_cycle(&superset, Some(&cached), true));

    let fresh = rework_list(&["fix-3", "fix-4"]);
    assert!(!replays_a_judged_cycle(&fresh, Some(&cached), true));
}

fn cycle_of(kind: PlanKind, cycle: u32, ids: &[&str], history: Vec<PlanCycle>) -> TaskPlan {
    TaskPlan {
        kind,
        cycle,
        history,
        ..plan_of(ids)
    }
}

fn greenfield_cycle(ids: &[&str]) -> PlanCycle {
    PlanCycle {
        cycle: 0,
        kind: PlanKind::Greenfield,
        tasks: plan_of(ids).tasks,
    }
}

fn ids(tasks: &[PlannedTask]) -> Vec<&str> {
    tasks.iter().map(|t| t.id.as_str()).collect()
}

#[test]
fn re_entering_an_unjudged_rework_cycle_keeps_its_cycle_and_history() {
    let cached = cycle_of(
        PlanKind::Rework,
        1,
        &["fix-1"],
        vec![greenfield_cycle(&["a", "b"])],
    );
    let out = plan_cache_entry(plan_of(&["fix-1"]), Some(&cached), false, true);
    assert_eq!(out.cycle, 1);
    assert_eq!(out.history.len(), 1);
    assert_eq!(ids(&out.history[0].tasks), ["a", "b"]);
    assert_eq!(ids(&out.already_landed), ["a", "b"]);
    assert!(out.resumes_landed_work);
    assert_eq!(out.kind, PlanKind::Rework);
}

#[test]
fn a_judged_rework_cycle_advances_the_cycle_and_closes_history() {
    let cached = cycle_of(
        PlanKind::Rework,
        1,
        &["fix-1"],
        vec![greenfield_cycle(&["a", "b"])],
    );
    let out = plan_cache_entry(plan_of(&["fix-2"]), Some(&cached), true, true);
    assert_eq!(out.cycle, 2);
    assert_eq!(out.history.len(), 2);
    assert_eq!(ids(&out.already_landed), ["a", "b", "fix-1"]);
    assert!(out.resumes_landed_work);
}

#[test]
fn a_rework_declaration_that_is_not_a_delta_never_loses_cached_history() {
    let cached = cycle_of(
        PlanKind::Rework,
        2,
        &["fix-2"],
        vec![greenfield_cycle(&["a"]), greenfield_cycle(&["fix-1"])],
    );
    let mut incoming = plan_of(&["x"]);
    incoming.kind = PlanKind::Rework;
    let out = plan_cache_entry(incoming, Some(&cached), true, false);
    assert_eq!(out.cycle, 2);
    assert_eq!(out.history.len(), 2);
    assert_eq!(ids(&out.tasks), ["x"]);
}

#[test]
fn a_first_rework_cycle_over_a_greenfield_cache_starts_at_cycle_one() {
    let cached = plan_of(&["a", "b"]);
    let out = plan_cache_entry(plan_of(&["fix-1"]), Some(&cached), true, true);
    assert_eq!(out.cycle, 1);
    assert_eq!(out.history.len(), 1);
    assert_eq!(ids(&out.already_landed), ["a", "b"]);

    let none = plan_cache_entry(plan_of(&["fix-1"]), None, false, true);
    assert_eq!(none.cycle, 1);
    assert!(none.history.is_empty());
}

// --- plan_epoch --------------------------------------------------------------
//
// The epoch is what the drill-down joins run rows on, so the one thing it
// must get right is which plans are the same lineage.

fn with_epoch(mut plan: TaskPlan, epoch: &str) -> TaskPlan {
    plan.epoch = Some(epoch.into());
    plan
}

fn fresh() -> String {
    "fresh".into()
}

#[test]
fn a_retry_of_the_same_list_keeps_the_epoch() {
    let cached = with_epoch(plan_of(&["a", "b"]), "e1");
    assert_eq!(
        plan_epoch(&plan_of(&["a", "b"]), Some(&cached), fresh),
        "e1"
    );
}

/// The bug this exists for: a re-run from an upstream step re-decomposed
/// into a new list that reused some of the old ids.
#[test]
fn a_new_decomposition_starts_a_new_epoch_even_when_ids_overlap() {
    let cached = with_epoch(
        cycle_of(
            PlanKind::Rework,
            3,
            &["fix-1"],
            vec![greenfield_cycle(&["a"])],
        ),
        "e1",
    );
    assert_eq!(
        plan_epoch(&plan_of(&["a", "b"]), Some(&cached), fresh),
        "fresh"
    );

    let cached = with_epoch(plan_of(&["a", "b"]), "e1");
    assert_eq!(
        plan_epoch(&plan_of(&["a", "c"]), Some(&cached), fresh),
        "fresh"
    );
    assert_eq!(
        plan_epoch(&plan_of(&["b", "a"]), Some(&cached), fresh),
        "fresh"
    );
}

/// A rework cycle is shown beside the cycles it is a delta against, so
/// their rows must stay joinable.
#[test]
fn a_rework_cycle_inherits_the_epoch() {
    let cached = with_epoch(plan_of(&["a", "b"]), "e1");
    let delta = plan_cache_entry(plan_of(&["fix-1"]), Some(&cached), true, true);
    assert_eq!(plan_epoch(&delta, Some(&cached), fresh), "e1");
}

#[test]
fn nothing_to_inherit_is_a_fresh_epoch() {
    assert_eq!(plan_epoch(&plan_of(&["a"]), None, fresh), "fresh");
    assert_eq!(
        plan_epoch(&plan_of(&["a"]), Some(&plan_of(&["a"])), fresh),
        "fresh"
    );
}

/// A restart mid-rework, end to end over the pure pieces: the consumer's
/// cycle-1 attempt cached its list and was interrupted, the restored retry
/// context puts it back in the rework cycle, and it re-reads that list.
/// Opening the resumed attempt closed the stale `running` row as
/// `interrupted`, so the cycle reads as unjudged — the same list is its
/// own cycle re-entered, not a replay, and the counter stays put.
#[test]
fn a_resumed_rework_cycle_is_neither_a_replay_nor_a_new_cycle() {
    use crate::domain::ids::StepExecutionId;
    use crate::domain::models::step_attempt::{cached_cycle_standing, StepAttempt};
    let attempt = |attempt_no: u32, status: &str| StepAttempt {
        step_execution_id: StepExecutionId::from("se-implement".to_string()),
        attempt_no,
        status: status.to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_ms: None,
        error_class: None,
        failure_fingerprint: None,
        applied_rule: None,
        workspace_fingerprint: None,
        idempotency_key: None,
        started_at: 0,
        ended_at: None,
    };
    let attempts = [
        attempt(1, "completed"),
        attempt(2, "interrupted"),
        attempt(3, "running"),
    ];
    let standing = cached_cycle_standing(&attempts, Some(2));
    assert!(!standing.judged);
    let judged = standing.judged;

    let cached = cache_after_one_rework();
    let incoming = rework_list(&["fix-1", "fix-2"]);
    assert!(!replays_a_judged_cycle(&incoming, Some(&cached), judged));
    assert!(is_rework_plan(&incoming, Some(&cached)));

    let out = plan_cache_entry(incoming, Some(&cached), judged, true);
    assert_eq!(out.cycle, 1);
    assert_eq!(out.history.len(), 1);
    assert_eq!(ids(&out.already_landed), ["a", "b", "c"]);
}

fn attempt_row(attempt_no: u32, status: &str) -> crate::domain::models::StepAttempt {
    crate::domain::models::StepAttempt {
        step_execution_id: crate::domain::ids::StepExecutionId::from("se-implement".to_string()),
        attempt_no,
        status: status.to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_ms: None,
        error_class: None,
        failure_fingerprint: None,
        applied_rule: None,
        workspace_fingerprint: None,
        idempotency_key: None,
        started_at: 0,
        ended_at: None,
    }
}

/// A replay refused between two cycles: attempt 1 ran cycle 1 to a
/// verdict, attempt 2 read the stale list and was sent back before it
/// cached anything, and the producer's corrected delta now arrives on
/// attempt 3. That delta is cycle 2, and cycle 1's tickets are landed work.
#[test]
fn a_delta_after_a_refused_replay_closes_the_judged_cycle() {
    use crate::domain::models::step_attempt::cached_cycle_standing;
    let attempts = [
        attempt_row(1, "completed"),
        attempt_row(2, "failed"),
        attempt_row(3, "running"),
    ];
    let standing = cached_cycle_standing(&attempts, Some(1));

    let cached = cache_after_one_rework();
    let incoming = rework_list(&["fix-3"]);
    assert!(!replays_a_judged_cycle(
        &incoming,
        Some(&cached),
        standing.replay_checkable
    ));
    let out = plan_cache_entry(incoming, Some(&cached), standing.judged, true);
    assert_eq!(out.cycle, 2);
    assert_eq!(out.history.len(), 2);
    assert_eq!(ids(&out.already_landed), ["a", "b", "c", "fix-1", "fix-2"]);
}

/// The producer, sent back once for a replay, hands the same list again.
/// That is its answer: it runs, rather than spending another redirect.
#[test]
fn a_replay_is_refused_only_once() {
    use crate::domain::models::step_attempt::cached_cycle_standing;
    let attempts = [
        attempt_row(1, "completed"),
        attempt_row(2, "failed"),
        attempt_row(3, "running"),
    ];
    let standing = cached_cycle_standing(&attempts, Some(1));
    let cached = cache_after_one_rework();
    let same = rework_list(&["fix-1", "fix-2"]);
    assert!(!replays_a_judged_cycle(
        &same,
        Some(&cached),
        standing.replay_checkable
    ));

    let first_time = cached_cycle_standing(&attempts[..1], Some(1));
    assert!(replays_a_judged_cycle(
        &same,
        Some(&cached),
        first_time.replay_checkable
    ));
}
