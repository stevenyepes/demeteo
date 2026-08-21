use serde::{Deserialize, Serialize};

/// One unit of work in a `sequence` step's task list.
///
/// Tasks run strictly in order, each in a *fresh* agent session but in the
/// *same* worktree, and each commits before the next starts. That ordering
/// is the whole point: task N sees task N-1's commits, so a later task can
/// legitimately build on an earlier one and `files` need not be disjoint
/// across tasks. (The old parallel design required disjoint ownership
/// precisely because its workers ran concurrently on separate worktrees and
/// had to be merged back independently.)
///
/// `files` is therefore advisory — it tells the agent what it is expected to
/// touch and drives the targeted-retry selection below. It is not enforced
/// as a filesystem fence: `Implement`-capability steps write across the tree
/// (builds, generated code, new files the planner did not foresee), and a
/// chmod fence over a necessarily-incomplete list would reject legitimate
/// work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTask {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub test_command: Option<String>,
    /// Binary pass/fail criteria for *this* task, rendered into the task
    /// agent's prompt as its done-definition. Optional: a legacy plan (or
    /// a task that is genuinely just "move the file") carries none.
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// Ids of earlier tasks this one builds on. Execution is strictly in
    /// list order either way — the edges exist so a targeted retry that
    /// re-runs a foundation task also re-runs the tasks stacked on it
    /// (see [`select_targeted_tasks`]), and so the validator can reject a
    /// list whose order contradicts its own dependencies.
    #[serde(default)]
    pub blocked_by: Vec<String>,
    /// Task-specific retry guidance. On a targeted retry the verdict's
    /// feedback is stamped here so a re-run task sees the guidance that
    /// actually concerns it rather than the whole verdict.
    #[serde(default)]
    pub retry_note: Option<String>,
}

/// Whether a task list decomposes the whole feature or only what a verdict
/// rejected.
///
/// Written by the producing step and read by the `sequence` step, which
/// treats the two completely differently: a greenfield list runs against a
/// worktree cut from a branch that carries none of it, while a rework list
/// runs against one that carries the entire previous cycle. Getting that
/// backwards is either an agent reimplementing code it is looking at, or a
/// delta applied to a tree that has nothing to apply it to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    /// The full decomposition. The default, so every task list written
    /// before this field existed — and every producer that never learned
    /// to set it — reads as what it is.
    #[default]
    Greenfield,
    /// A delta closing a downstream verdict. Every task runs; the previous
    /// cycle's tasks are reported as already landed.
    Rework,
}

/// An ordered task list: either authored upstream (the decomposition step
/// writes it as a declared artifact, so a human gate can review it before
/// any code is written) or, for legacy `parallel` workflows that declare no
/// `task_list_from`, produced by a planner turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskPlan {
    /// Accepts `tasks` (the current schema) or `subtasks` (what the old
    /// planner prompt emitted), so a legacy workflow's planner output still
    /// parses.
    #[serde(alias = "subtasks")]
    pub tasks: Vec<PlannedTask>,
    /// Whole decomposition, or a delta against work already on the branch.
    ///
    /// Authoritative when the producer sets it. When it doesn't — a
    /// hand-written list, an older prompt — the sequence step falls back to
    /// comparing this list's ids against the previous cycle's, which
    /// answers the same question from evidence rather than declaration.
    #[serde(default)]
    pub kind: PlanKind,
    /// 0 for the original decomposition, incrementing once per rework
    /// cycle. Assigned by the `sequence` step as it resolves the plan, not
    /// by the producer — the producer has no way to know how many cycles
    /// preceded it, and a number it guessed would silently mislabel the
    /// history.
    #[serde(default)]
    pub cycle: u32,
    /// Earlier cycles of this same step, oldest first, each carrying the
    /// tasks it decomposed.
    ///
    /// Kept so a rework cycle does not erase the decomposition it is a
    /// delta against: the drill-down renders every cycle, and the tasks a
    /// running agent must be told are already on the branch are read from
    /// here. Empty for a greenfield plan, which is why it skips
    /// serializing — an untouched row's JSON is byte-identical to what it
    /// was before this field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<PlanCycle>,
    /// Tasks a targeted retry is deliberately *not* re-running because their
    /// work is already committed on the feature branch (see
    /// [`select_targeted_tasks`]).
    ///
    /// They are not executed, but they must still be named in each running
    /// task's `{{completed_tasks}}`: the worktree the agent opens contains
    /// their work, and a prompt that says "None — this is the first task"
    /// while the tree holds three tasks' worth of code is precisely the lie
    /// the completed-task record exists to prevent.
    ///
    /// Never serialized — this is execution state, not part of the task-list
    /// contract an upstream step writes.
    #[serde(skip)]
    pub already_landed: Vec<PlannedTask>,
    /// True when this plan is a retry of an attempt whose work is still on the
    /// feature branch, so the worktree the tasks open is *not* empty.
    ///
    /// `already_landed` covers the tasks this attempt skips, but a retry that
    /// re-runs every task (the verdict implicated no files, or none matched)
    /// leaves it empty while the tree still holds the whole previous attempt.
    /// The prompt has to say so either way, or the first task is told it is
    /// starting from nothing.
    #[serde(skip)]
    pub resumes_landed_work: bool,
    /// The producer's prose reason for the list it wrote, carried through
    /// from the artifact.
    ///
    /// It exists for one case, and that case is the whole justification: a
    /// rework producer is *told* to emit no tickets when the review it is
    /// scoping named nothing an implementation ticket can fix ("Say so in
    /// the ticket list's absence rather than inventing work"). Its reason
    /// for doing so is the only thing that makes that outcome legible to a
    /// human, and before this field `serde` silently dropped it as an
    /// unknown key — leaving a zero-task list that looked indistinguishable
    /// from a broken one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// One finished cycle of a step's decomposition, as stored in
/// [`TaskPlan::history`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCycle {
    pub cycle: u32,
    #[serde(default)]
    pub kind: PlanKind,
    pub tasks: Vec<PlannedTask>,
}

impl TaskPlan {
    /// Every task this step has ever planned, oldest cycle first, followed
    /// by this plan's own.
    ///
    /// The record a rework cycle's agents are shown as "already committed
    /// on your branch". Ordered rather than set-like because the prompt
    /// renders it as a list an agent reads top to bottom, and the order the
    /// tasks landed in is the order that makes it legible.
    pub fn all_prior_tasks(&self) -> Vec<PlannedTask> {
        self.history
            .iter()
            .flat_map(|c| c.tasks.iter())
            .cloned()
            .collect()
    }

    /// Every task this plan accounts for, `history` **plus its own**.
    ///
    /// The distinction from [`Self::all_prior_tasks`] is easy to get wrong
    /// and expensive when you do. Asked of the *cached* plan, this is "every
    /// ticket the previous cycles planned" — which is what an incoming list
    /// must be compared against to tell a delta from a re-decomposition.
    /// `all_prior_tasks` would answer that with `history` alone, and on the
    /// first rework cycle `history` is still empty while the twenty-five
    /// original tickets sit in `tasks`: the comparison would find nothing to
    /// overlap with, read an undeclared delta as greenfield, and re-run
    /// every one of them.
    pub fn all_planned_tasks(&self) -> Vec<PlannedTask> {
        let mut out = self.all_prior_tasks();
        out.extend(self.tasks.iter().cloned());
        out
    }

    /// Fold this plan into `history` as a completed cycle, returning the
    /// history the *next* cycle starts from.
    ///
    /// Only the tasks survive: the execution-state fields
    /// (`already_landed`, `resumes_landed_work`) describe one attempt, not
    /// the decomposition, and carrying them forward would let a stale
    /// attempt's bookkeeping outlive it.
    pub fn close_cycle(&self) -> Vec<PlanCycle> {
        let mut history = self.history.clone();
        history.push(PlanCycle {
            cycle: self.cycle,
            kind: self.kind,
            tasks: self.tasks.clone(),
        });
        history
    }
}

/// The task-list JSON shape, as shown to a planner/spec agent and (in the
/// `include_retry_note: false` form) surfaced in the "could not read a task
/// list" error message.
///
/// A single source: every prompt or error text that needs to describe
/// `PlannedTask`'s shape calls this instead of hand-typing the JSON example,
/// so adding, renaming, or removing a field is one edit instead of several
/// separately hand-maintained string literals that can — and did — drift out
/// of sync (the fenced examples in `plan.rs` and the standard pipeline's
/// `s-tickets` prompt already used a different sample id, `task-1` vs
/// `ticket-1`, before this existed).
pub(crate) fn task_list_json_shape_example(include_retry_note: bool) -> String {
    let mut fields = vec![
        r#""id": "task-1""#.to_string(),
        r#""title": "...""#.to_string(),
        r#""description": "...""#.to_string(),
        r#""files": ["src/foo.rs"]"#.to_string(),
        r#""test_command": "...""#.to_string(),
        r#""acceptance": ["..."]"#.to_string(),
        r#""blocked_by": []"#.to_string(),
    ];
    if include_retry_note {
        fields.push(r#""retry_note": null"#.to_string());
    }
    format!("{{\"tasks\": [{{{}}}]}}", fields.join(", "))
}

/// Reject a task list that would misbehave at execution time.
///
/// The task list crossed a trust boundary when it moved out of the step and
/// into an artifact an agent writes: `serde` proves it is shaped like a plan,
/// not that it is a *sane* one. Each rule below maps to a concrete failure:
///
/// * **empty / blank ids** — the id keys the agent session (`{feature}-{step}-
///   {task}`) and names the task in `completed_tasks`; blank makes both
///   ambiguous.
/// * **duplicate ids** — two tasks sharing an id collide on that session key,
///   and [`select_targeted_tasks`] would treat them as one task when deciding
///   what to re-run and what to report as already landed.
/// * **`blocked_by` pointing forward or at nothing** — tasks run strictly in
///   list order, so a dependency on a later (or missing) task is an ordering
///   the executor cannot honor; the "dependency" would run *after* the task
///   that needs it.
///
/// There is deliberately no cap on task count. Decomposition is sized by the
/// ticket rubric (one context window per task); a hard count limit only
/// punished well-decomposed large features. Total cost is bounded instead, by
/// the aggregate dollar ceiling `run_tasks_loop` enforces across the whole
/// list (`SEQUENCE_STEP_COST_CEILING_MULTIPLIER`, in the sequence step's
/// `runner.rs`) — the
/// per-task budget alone resets every task and does not, on its own, bound
/// anything about the list as a whole.
///
/// Returns a human-readable reason, or `None` when the plan is executable.
pub(crate) fn validate_task_plan(plan: &TaskPlan) -> Option<String> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (i, task) in plan.tasks.iter().enumerate() {
        let id = task.id.trim();
        if id.is_empty() {
            return Some(format!(
                "task at position {} has an empty `id`. Every task needs a unique, stable, \
                 kebab-case id.",
                i + 1
            ));
        }
        if !seen.insert(id) {
            return Some(format!(
                "task id '{}' appears more than once. Task ids key the agent session and the \
                 completed-task record, so they must be unique.",
                id
            ));
        }
        for dep in &task.blocked_by {
            let dep = dep.trim();
            if dep.is_empty() {
                continue;
            }
            // `seen` holds exactly the ids of earlier tasks (this task's id
            // included, which also catches a self-dependency).
            if dep == id {
                return Some(format!(
                    "task '{}' lists itself in `blocked_by`. A task cannot depend on itself.",
                    id
                ));
            }
            if !seen.contains(dep) {
                return Some(format!(
                    "task '{}' is blocked_by '{}', which is not an earlier task in the list. \
                     Tasks run strictly in list order, so every dependency must appear before \
                     the task that needs it.",
                    id, dep
                ));
            }
        }
    }
    None
}

/// Build the attempt-1 targeted retry plan from the cached full plan.
///
/// Selects the tasks whose `files` intersect the verdict's
/// `implicated_files`, plus — transitively — every task that either
/// `blocked_by` a selected one or shares a `files` entry with one: re-running
/// a foundation task rewrites what its dependents were built on, so they
/// re-run too rather than being reported as landed work that still matches
/// the branch. The file-overlap leg exists because `blocked_by` is a
/// planner-declared edge and can be incomplete — a task that touches the
/// same file as a re-run task is re-running-worthy whether or not the
/// planner remembered to say so. Stamps the retry feedback onto each
/// selected task as its `retry_note`. Falls back to re-running every task
/// when the verdict named no files (or none matched) — a blind retry is
/// still correct, just not cheap.
///
/// Re-running a subset is safe here in a way it was not under the parallel
/// design: the tasks that are *not* re-run already have their commits on
/// the feature branch, so skipping them preserves their work instead of
/// dropping it. They come back in [`TaskPlan::already_landed`], because the
/// running tasks' prompts have to say so — the worktree contains their work
/// whether or not this attempt re-runs them.
///
/// This only holds when the previous attempt's merge actually landed, which
/// is why the caller restricts it to failures raised by a *later* step; a
/// sequence step that failed on its own rolls its commits back.
///
/// Implemented as one forward pass over `cached.tasks`: `blocked_by` only
/// ever references an earlier task (validated) and file-overlap can only
/// pull in a later task from an earlier one by the same reasoning (the
/// earlier task cannot have been "built on" a file a later task introduces),
/// so a task's selection is fully decided by the tasks already visited.
/// That also means `selected` comes out already in list order — the
/// executor's contract — with no separate sort or already-landed pass
/// needed.
pub(crate) fn select_targeted_tasks(
    cached: &TaskPlan,
    feedback: &str,
    implicated_files: &[String],
) -> TaskPlan {
    fn norm(p: &str) -> String {
        p.trim().trim_start_matches("./").to_string()
    }
    let implicated: Vec<String> = implicated_files
        .iter()
        .map(|s| norm(s))
        .filter(|s| !s.is_empty())
        .collect();

    let owns_implicated = |task: &PlannedTask| -> bool {
        task.files.iter().any(|f| {
            let f = norm(f);
            implicated.iter().any(|i| {
                *i == f || i.ends_with(&format!("/{}", f)) || f.ends_with(&format!("/{}", i))
            })
        })
    };

    // The verdict's files told us nothing usable — either it named none, or
    // it named files nothing in the plan owns — so every task is re-run. A
    // blind retry, but still a correct one.
    let blind_retry = implicated.is_empty() || !cached.tasks.iter().any(owns_implicated);

    let mut selected_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut selected_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut selected: Vec<PlannedTask> = Vec::new();
    let mut already_landed: Vec<PlannedTask> = Vec::new();
    for task in &cached.tasks {
        // Ids are compared trimmed: validate_task_plan accepts a
        // whitespace-padded id (it trims both sides), so an untrimmed
        // lookup here would silently fail to recognize an otherwise-valid
        // dependency.
        let task_id = task.id.trim();
        let blocked_by_a_selected_task = task
            .blocked_by
            .iter()
            .any(|dep| selected_ids.contains(dep.trim()));
        let shares_a_selected_file = task
            .files
            .iter()
            .map(|f| norm(f))
            .any(|f| !f.is_empty() && selected_files.contains(&f));

        if blind_retry
            || owns_implicated(task)
            || blocked_by_a_selected_task
            || shares_a_selected_file
        {
            selected_ids.insert(task_id);
            selected_files.extend(task.files.iter().map(|f| norm(f)).filter(|f| !f.is_empty()));
            let mut task = task.clone();
            task.retry_note = Some(feedback.to_string());
            selected.push(task);
        } else {
            already_landed.push(task.clone());
        }
    }

    TaskPlan {
        tasks: selected,
        already_landed,
        resumes_landed_work: true,
        ..cached.clone()
    }
}

/// Is `incoming` a delta against `previous`, or a fresh whole decomposition?
///
/// The producing step declares it ([`PlanKind`]) and that declaration wins.
/// The fallback exists because the declaration is written by an agent
/// following a prompt, and a prompt is not a schema: a producer that emits
/// a rework list without the marker would otherwise be read as a greenfield
/// list, and the `sequence` step would tell its agents the branch is empty
/// while they stand in a worktree holding the whole previous cycle.
///
/// The evidence is id overlap. A greenfield re-decomposition reissues the
/// same ticket ids (the producer is revising a list it is looking at); a
/// delta names work that did not exist before. So **no shared id with the
/// previous cycle** reads as rework.
///
/// Two guards keep that from firing on the wrong thing. An empty previous
/// cycle shares no ids with anything, and a first run has no previous cycle
/// at all — both are greenfield by definition. And the caller only consults
/// this when the run is *already* in a rework cycle by graph position
/// ([`crate::domain::rework`]), so the worst a wrong answer here can do is
/// re-run a list that would otherwise have been skipped — never the
/// reverse.
///
/// `previous` is the whole cached [`TaskPlan`], not a task slice, so the
/// comparison set is chosen here rather than at the call site. Handing the
/// caller that choice is how this gets silently broken: `all_prior_tasks`
/// looks like the right answer and is empty on the first rework cycle,
/// where the tickets to compare against are the cached plan's *own*
/// (see [`TaskPlan::all_planned_tasks`]).
pub fn is_rework_plan(incoming: &TaskPlan, previous: Option<&TaskPlan>) -> bool {
    if incoming.kind == PlanKind::Rework {
        return true;
    }
    let Some(previous) = previous else {
        return false;
    };
    let previous = previous.all_planned_tasks();
    if previous.is_empty() || incoming.tasks.is_empty() {
        return false;
    }
    let prior: std::collections::HashSet<&str> = previous.iter().map(|t| t.id.trim()).collect();
    !incoming.tasks.iter().any(|t| prior.contains(t.id.trim()))
}

/// Drop the tasks a mid-list checkpoint already landed on the feature branch.
///
/// When a task fails partway through the list, the step merges the completed
/// prefix before failing (see the adapter's `handle_sequence_step`), so the
/// next attempt
/// must not re-run — and re-pay for — tasks whose commits are already on the
/// branch. The landed tasks move into [`TaskPlan::already_landed`] so the
/// running tasks' prompts still describe the tree they open.
///
/// Applied to *every* resolved plan, whatever attempt produced it: a
/// checkpoint only exists while its work is on the branch, and the caller
/// clears it the moment the step completes or the branch is rolled back.
///
/// A checkpoint covering *every* task leaves nothing to run, and that is a
/// real state rather than a broken one: since V35 the task loop checkpoints
/// each task as it commits, so the row names the whole plan from the moment
/// the last task lands until the step completes — a window that spans the
/// declared-artifact check, the verifier's agent pass, and the final merge.
/// A kill in there must cost the *merge*, not twenty-five agents.
///
/// This used to put the full plan back, on the premise that all-ids-matched
/// meant a stale row. That premise died with V32: the caller now resolves a
/// [`CheckpointResume`](crate::adapters::step_executor::steps::sequence::CheckpointResume) first, which verifies the
/// anchor against the repo, so by the time a plan reaches this filter the
/// landed work is known to be either merged or about to be restored.
///
/// What that trades away: a redirect that revises task *bodies* while keeping
/// their ids is now indistinguishable from "already done" and will be skipped.
/// `replay_from_step` is the escape hatch — it clears the checkpoint, which is
/// exactly what an explicit redo should do.
pub(crate) fn apply_landed_checkpoint(mut plan: TaskPlan, landed_ids: &[String]) -> TaskPlan {
    let landed_set: std::collections::HashSet<&str> =
        landed_ids.iter().map(|s| s.as_str()).collect();

    let (landed, remaining): (Vec<PlannedTask>, Vec<PlannedTask>) = plan
        .tasks
        .into_iter()
        .partition(|t| landed_set.contains(t.id.as_str()));

    if landed.is_empty() {
        plan.tasks = remaining;
        return plan;
    }

    plan.tasks = remaining;
    plan.already_landed.extend(landed);
    plan.resumes_landed_work = true;
    plan
}

/// Best-effort extractor for a task plan.
///
/// Serves two callers with the same tolerance, deliberately: the artifact
/// path (where `text` is the literal contents of `artifacts/task-list.json`,
/// which *should* be bare JSON but which an agent may still have wrapped in
/// a fence or preceded with prose) and the legacy planner path (where `text`
/// is a whole agent turn). Tries, in order: a ```json fence, any fence, then
/// the first balanced top-level `{...}`. Returns the first object that
/// deserializes as a [`TaskPlan`].
pub(crate) fn extract_task_plan(text: &str) -> Option<TaskPlan> {
    // 0) The happy path for a task-list artifact: the whole thing is JSON.
    if let Ok(d) = serde_json::from_str::<TaskPlan>(text.trim()) {
        return Some(d);
    }
    // 1) ```json ... ``` fence
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Ok(d) = serde_json::from_str::<TaskPlan>(body) {
                return Some(d);
            }
        }
    }
    // 2) Generic ``` ... ``` fence (any language tag)
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        // skip optional language tag on the same line
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Ok(d) = serde_json::from_str::<TaskPlan>(body) {
                return Some(d);
            }
        }
    }
    // 3) Top-level JSON object (find balanced braces)
    if let Some((start, end)) = find_top_level_object(text) {
        if let Ok(d) = serde_json::from_str::<TaskPlan>(&text[start..end]) {
            return Some(d);
        }
    }
    None
}

/// Find the (start, end) indices of the first top-level `{...}` object in
/// `s`. `end` is exclusive (i.e. one past the matching `}`).
pub(crate) fn find_top_level_object(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
                if depth == 0 {
                    if let Some(st) = start {
                        if st < i {
                            return Some((st, i + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Commitlint's header limits, as `@commitlint/config-conventional` sets
/// them out of the box.
///
/// Demeteo cannot read the *target* repo's `.commitlintrc` — it may not have
/// one, and reading it would mean parsing a JS config from a worktree — so
/// these are the defaults, and they are the right guess: a project that
/// lints commits at all almost always extends config-conventional, and a
/// project that lints nothing is unharmed by a message that would have
/// passed.
const SUBJECT_MAX: usize = 72;
const HEADER_MAX: usize = 100;
/// Classic git body width, comfortably under config-conventional's
/// `body-max-line-length` of 100 — which a long title *would* trip if the
/// overflow were emitted as one line.
const BODY_WRAP: usize = 72;

/// The commit message for one finished task.
///
/// Every task in a `sequence` step commits before the next one starts, and
/// those commits land on the feature branch — inside the very range
/// (`<default>..HEAD`) that the target repo's own harness lints. So this
/// message is not cosmetic: an over-long subject fails the project's
/// `commitlint` run, which fails its test harness, which fails the validate
/// step, which opens a rework cycle — whose ticket title becomes the next
/// over-long subject. The loop is self-feeding and no ticket can break it,
/// because the defect is in the orchestrator's own commit, not in the code
/// under review. Hence the truncation here rather than a check somewhere
/// that could only report the problem after the fact.
///
/// Task titles are written by an agent against a rubric that never mentioned
/// a length limit, so treating a long one as an error would fail a step over
/// a naming choice. The title is shortened for the subject and preserved in
/// full in the body instead.
pub fn task_commit_message(feature_id: &str, task_id: &str, title: &str) -> String {
    let mut full = normalize_subject(title);
    if full.is_empty() {
        full = normalize_subject(task_id);
    }
    if full.is_empty() {
        full = "implement task".to_string();
    }

    // Both limits bind, and the header's is the tighter one once the scope
    // is long enough. `feat(): ` is the 8 fixed characters around the scope.
    // The floor keeps an absurd scope from producing an empty subject — that
    // header is over the limit whatever we do, and a truncated subject is
    // still more useful than none.
    let cap = SUBJECT_MAX
        .min(HEADER_MAX.saturating_sub(feature_id.chars().count() + "feat(): ".len()))
        .max(20);

    let subject = truncate_on_word_boundary(&full, cap);
    let header = format!("feat({}): {}", feature_id, subject);
    if subject == full {
        header
    } else {
        format!("{}\n\n{}", header, wrap_body(&full, BODY_WRAP))
    }
}

/// Fold an agent-written title into something a conventional-commit subject
/// line accepts: one line, lower case (commitlint's `subject-case` rejects
/// sentence/start/pascal/upper), no trailing period (`subject-full-stop`).
fn normalize_subject(title: &str) -> String {
    let collapsed = title.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .to_lowercase()
        .trim_end_matches(['.', ' '])
        .to_string()
}

/// Cut `s` to at most `cap` characters, preferring the last word boundary.
///
/// Falls back to a hard cut when the boundary would leave less than half the
/// budget — a two-word subject truncated to one word says less than a
/// mid-word cut does.
fn truncate_on_word_boundary(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_string();
    }
    let end = s.char_indices().nth(cap).map(|(i, _)| i).unwrap_or(s.len());
    let head = &s[..end];
    let cut = match head.rfind(' ') {
        Some(i) if i >= cap / 2 => i,
        _ => head.len(),
    };
    let out = head[..cut].trim_end_matches([' ', '.', ',', ';', ':', '-']);
    if out.is_empty() {
        head.to_string()
    } else {
        out.to_string()
    }
}

/// Greedy word wrap. A single word longer than `width` gets its own
/// over-long line rather than being cut — the body is prose for a human,
/// and an unbroken token (a path, a symbol) is worth more intact.
fn wrap_body(s: &str, width: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if line.is_empty() {
            line.push_str(word);
        } else if line.chars().count() + 1 + word.chars().count() <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            lines.push(std::mem::take(&mut line));
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "../../../tests/domain/sequence/tasks.rs"]
mod targeted_retry_tests;
