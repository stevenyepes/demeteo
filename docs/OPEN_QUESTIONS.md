# Demeteo: Open & Deferred Questions

> **Captured here so they don't get lost.** These are the questions and
> features that came up during the multi-agent orchestrator redesign
> interview but were explicitly deferred to a future version. When a
> future phase picks one of these up, move the relevant entry from this
> doc into the active plan.

## Phase Placement Key

- **v1.0** — current scope, shipping in the first multi-agent release.
- **v1.1** — first follow-up; ships within 1–2 release cycles of v1.0.
- **v1.2** — second follow-up.
- **v1.x** — any 1.x release; no specific commit promised.
- **v2+** — major version; not before the multi-agent orchestrator is stable.
- **v3+** — far future; reconsidered only if product survives that long.

---

## 1. Multi-feature concurrency (Q18 → **answered 2026-07-12**)

**The question:** Can a project run 2+ features in parallel? With what resource limits?

**The answer:** Yes — N features per project, concurrently. See [decision 18](DECISIONS.md#2-superseded-decisions), which supersedes the original "strict serial" answer and records why.

> The old answer was *strict serial — one feature per project at a time, with a single "Current feature" slot + a queued list*. It was never actually enforced in code, so features already ran concurrently; the change makes the intent honest and the code safe.

**What "correctness" means here.** Concurrent features are safe exactly as long as they share nothing mutable. Git already satisfies this (it locks per-ref, and features touch disjoint refs), and worktree directories are feature-scoped. The one rule to keep upholding:

> **Share content-addressed *download* caches; never share *build* output.** Download caches (Cargo registry, npm `_cacache`, pip wheels) are immutable-by-content. Build/install outputs (`node_modules`, `target`, `.venv`, `.next`) are per-branch state and must be per-feature — a shared one lets feature B's install decide feature A's harness verdict, which then drives A's retry and critic loops.

**Still open — the resource ceiling.** Correctness is settled; *volume* is not. N features × M sequence tasks × one agent process each is unbounded. Two independent axes:

- **Per project** (how many features on one repo) — `max_concurrent_features`, default 2.
- **Cross project** (how many anywhere) — bounded by CPU / RAM / agent processes, not by correctness. A global cap.

Plus the budget and scheduling machinery that was always part of this: per-project `max_concurrent_llm_spend_usd_per_hour`; a feature-queue panel with promote/demote; resource-contention monitoring (worktree disk, agent sessions) with auto-pause on exhaustion.

---

## 2. Workflow YAML view (Q19-B → **revised 2026-07-23**)

**The question:** How does the user *write* a workflow?

**The answer now:** The visual DAG builder (`PRD_DAG_WORKFLOWS.md`) supersedes the form editor entirely, and [decision 42](DECISIONS.md#1-the-locked-decisions) ships a **read-only Monaco JSON source tab** in builder Phase 3. See the superseded entry for [decision 19](DECISIONS.md#2-superseded-decisions).

**Still deferred:** the *editable* source view with two-way canvas ↔ source binding, autocompletion against the published JSON Schema, inline validation. Requires a new decision record before it's picked up.

---

## 3. "Save this run as a workflow template" (Q19-C → v1.2)

**The question:** Can a user bootstrap a workflow from a past execution?

**The v1.0 answer:** No. Users start from the starter pack or a blank form.

**The deferred work:** On any completed feature, a "Save as workflow template" action. Demeteo invokes the planner (or a dedicated LLM call) to inspect the step sequence, prompts, agent configs, and timing; produces a draft workflow JSON. The user reviews in the form editor, edits, saves.

**Why deferred:** Needs a planner-as-author LLM call (cost + latency on the bootstrap step). Better to ship when we have v1.1's form ↔ YAML round-trip so the generated workflow can be inspected in either form.

---

## 4. Deep dry-run (Q23-C → v1.x)

**The question:** Can the user dry-run a workflow to see the full planning output before launching?

**The v1.0 answer:** Static pre-flight only (step list + risks + repo fit; no LLM call).

**The deferred work:** A "Deep dry-run" action in project settings that creates a real feature with `dry_run: true`, runs the workflow's `agent` steps in read-only mode, captures the output, presents it to the user, then discards. The read-only mode needs careful design (what if the agent tries to write? we mock the worktree).

**Why deferred:** Expensive (real LLM calls, possibly many) and slow (5–10 minutes for a realistic dry-run). Destroys the "just describe and launch" UX if it ran on every launch. Better as an opt-in for cautious users.

---

## 5. Cost rollup dashboard (Q21-C → v1.x)

**The question:** Does the user see aggregated cost across features?

**The v1.0 answer:** Per-step cost + duration, surfaced in step timeline and feature header. No project-level rollup.

**The deferred work:** A project overview panel showing "this month: $X across N features, M steps, top 3 expensive workflows." Aggregations across features in a project. Per-feature history list with click-through to the feature detail view.

**Why deferred:** Strict serial means the "current feature" view dominates. Aggregations across multiple features become valuable when multi-feature concurrency (Q1 above) lands.

---

## 6. Smart project home with activity feed (Q21-D → deferred)

**The question:** Should the project home include a project health panel and an activity feed?

**The v1.0 answer:** No. Project home is current feature + queue + repo map (Q21-B).

**The deferred work:** A project health panel ("3 features completed this month, 1 conflict resolved, 0 unresolved conflicts, $X spent") and a recent-activity feed. "Smart home" feel.

**Why deferred:** Activity feed needs careful filtering or it becomes unreadable. Health panels can become theater. Better to wait for real user feedback on what the home view needs.

---

## 7. Project tabs / split view / activity feed home (Q24-B/C/D → deferred)

**The question:** How does the user navigate between projects?

**The v1.0 answer:** Left rail with project list, main pane = current project (Q24-A).

**The deferred work:**
- **Tabs** (B): horizontal tab bar; user can have N project tabs open.
- **Split view** (C): 2–4 panes, each showing a different project.
- **Activity feed home** (D): landing screen is a global activity feed; project home is one click away.

**Why deferred:** Tabs add lifecycle management. Split view fights the strict-serial model. Activity feed inverts the natural mental model. The left rail is enough for v1.

---

## 8. `command` step type (Q8-B → **closed 2026-07-26**)

**The question:** Can a workflow step be a deterministic shell command instead of an LLM agent?

**The v1.0 answer:** No. The three step types are `agent`, `parallel`, `gate`. `command` is a useful type but is really a special case of `agent` (spawn an agent and have it run commands).

**Shipped (task P3.5, 2026-07-26):** a first-class `command` node type — `command`, `cwd`, `env_allowlist`, `timeout_secs`, `idempotent` — running via the existing `ExecutionPort` in a disposable worktree at zero token cost. Zero-token harness/build/script steps no longer need an agent prompt to emulate them. It landed as the node-type-registry extensibility proof: one registration line, no scheduler or dispatch edit, and no frontend source change at all (the palette and config panel derive from the registry). Two boundaries it deliberately keeps: it never merges its worktree back (a step that changes tracked files is an `agent` step), and a command not declared `idempotent` always parks at the Decision-14 synthetic gate when interrupted rather than being re-run. See `docs/PRD_DAG_WORKFLOWS.md` §5.2 and `steps/command.rs`.

---

## 9. WASM plugin host (legacy ARCHITECTURE design → deferred)

**The question:** Can third parties ship custom approval logic, telemetry integrations, or cross-cutting policy as WASM plugins?

**The v1.0 answer:** No. The four-axis `PermissionProfile` (`read_fs | write_fs | execute | network`) plus the `WriteScope`-driven chmod fence cover all v1 needs.

**The deferred work:** A WASM plugin host loaded from `~/.config/demeteo/plugins/`, evaluated inside a `wasmtime` sandbox. Plugins can hook the approval / telemetry / policy points exposed by the host.

**Why deferred:** Not needed for v1. Becomes valuable when third parties want to ship custom logic without rebuilding demeteo. The legacy `docs/LEGACY_ARCHITECTURE.md` referenced in the original wording does not exist in this repository; the spec should be re-derived if/when the host is built.

---

## 10. Per-machine / per-project agent config (existing v1 design → partial)

**The question:** Can the user configure an agent's model, working directory, and environment variables per machine?

**The v1.0 answer:** *Partial.* Per-project defaults live on `ProjectSettings::default_agent_kind` and `default_model`. Per-workflow overrides live on `ProjectWorkflowOverride` rows (`step_id = None` for workflow-level, `Some(step_id)` for step-level). Per-feature overrides live on the feature row (`Feature::agent_kind`, `Feature::model`, `Feature::step_overrides`); `step_overrides` is no longer a launch-time snapshot — a step's agent/model/effort is re-pointable while the run is alive, locally and on a detached runner ([decision 53](DECISIONS.md#53--mid-run-assignment-detail)). A `WorkingMemoryEntry` shape exists for thread-scoped working memory. The per-machine `AgentProfile` rows exist for the legacy shell / custom-http agent kinds (ollama, openai, cli, custom_http) and are still managed in `Machine` / `AgentProfile`. Demeteo does **not** read, store, or inject the LLM API key itself; the user configures the agent on the host (decision 4).

**The deferred work:** A first-class structured `AgentConfig { kind, model, workdir, env_refs, model_pricing_override }` per machine, editable from `EnvModal` (or its successor). The per-step half of this closed with decision 53; what remains of it is **cross-surface live refresh** — an assignment changed in one window does not reach another open inspector on the same run until that inspector reloads. Deliberately deferred rather than overlooked: the fix is a `DomainEvent` variant plus a `useRunEvents` subscription, which costs an exhaustive-match arm, a notifier category and a run-event vocabulary entry for a refresh nobody has asked for. Additive, so it can land on its own.

**Why deferred:** Users already configure their agents. Demeteo managing this duplicates the agent's own config UX. Defer until there's a clear reason.

---

## 11. Telemetry (Q31 → v3+)

**The question:** Does demeteo collect anonymous usage data?

**The v1.0 answer:** No. No telemetry of any kind.

**The deferred work:** A "Help improve demeteo" opt-in in Preferences → Defaults, with a clear disclosure of "data we would collect" (e.g., feature counts, workflow step counts, error rates, never file paths or content). Off by default; explicit opt-in required.

**Why deferred:** Demeteo has no users yet. Telemetry is a v3+ concern that requires a privacy review and a clear data-handling story.

---

## 12. Auto-update (Q30 → v2+)

**The question:** Does demeteo update itself?

**The v1.0 answer:** No. Users download new binaries manually.

**The deferred work:** Auto-update via the Tauri updater or Sparkle. Configurable update channel (stable / beta). Update notifications on launch.

**Why deferred:** Auto-update is a significant operational concern (CDN, signing, rollback, staged rollouts). v1.x is small enough that manual downloads are fine.

---

## 13. WASM provider plugins (Q4-C → v2+)

**The question:** Can third parties ship provider instances (GitHub Enterprise, Bitbucket, Gitea, etc.) as WASM plugins?

**The v1.0 answer:** No. v1 ships with GitHub and GitLab hard-coded.

**The deferred work:** A `ProviderInstance` trait that the GitHub and GitLab adapters implement. Third parties can implement the same trait in a WASM module and load it from `~/.config/demeteo/providers/`.

**Why deferred:** Not needed until the user base has provider needs beyond GitHub/GitLab.

---

## 14. Second non-CLI runtime (Anthropic → v1.1)

**The question:** What if the planner's host doesn't have any of the CLI agents?

**The v1.0 answer:** The planner is one of the CLI coding agents (opencode, hermes, claude-code). All speak CLI mode with `--format json` / `--print --output-format stream-json`. The runtime trait (`AgentRuntime` + `AgentSession`) is transport-neutral; the `UnifiedCliRuntime` impl handles them all.

**The deferred work:** A non-CLI adapter for an agent that doesn't ship a CLI. Candidates: the `opencode serve` HTTP API (real-time permission approval via `POST /session/:id/permissions/:permissionID`), or a raw Anthropic API for a custom planner. The runtime trait surface already supports these (the per-adapter `perm_env` and `build_args` are the only knobs).

**Why deferred:** All four v1 agents speak CLI mode. The "second adapter must be non-CLI" rule from the original design interview is a v1.1 commitment, not a v1 requirement. (ACP itself was removed per decision 34; the deferred item is moot in that framing.)

---

## 15. Self-hosted provider instance key rotation (operational → v1.x)

**The question:** What happens when a self-hosted GitLab instance rotates its PAT signing key?

**The v1.0 answer:** The user manually disconnects and reconnects. The old encrypted PAT is lost; the user pastes a new one.

**The deferred work:** A "rotate key" affordance in `ProviderSettings` that re-validates the new key without dropping the connection. Background re-validation on a schedule.

**Why deferred:** Not a v1 critical path. Self-hosted GitLab key rotation is infrequent.

---

## 16. Telemetry-free observability (Q16-C/D → partially deferred)

**The question:** Does the user see project-level rollups and budget guardrails?

**The v1.0 answer:** Per-step cost + duration (Q16-B). No project rollup, no budget guardrails.

**The deferred work:** Q16-C (project rollup) → v1.x. Q16-D (budget guardrails) → v1.x.

**Why deferred:** Strict serial means the "current feature" view is enough for v1. Rollups + budgets become valuable with multi-feature concurrency.

---

## 17. Other captured items

These came up briefly but weren't deep-dived in the interview. Captured here for completeness:

- **SSH agent on macOS** — keyring integration beyond `keyring` crate; defer to a v1.x polish item.
- **Per-step cost cap** at the step level (not just the retry level) — useful for runaway steps; defer to v1.x.
- **Workflow marketplace / community sharing** — defer until there's a community; v3+ at earliest.
- **Custom step types via WASM** — defer with the WASM plugin host.
- **Pluggable UI themes beyond the dark neon system** — defer; the design system is the product identity.
- **Mobile / web companion** — explicitly out of scope; demeteo is a desktop control plane.
- **WebKitGTK + NVIDIA + Wayland Error 71** — not a deferred feature, but a documented platform quirk with an auto-detected workaround and `DEMETEO_DISABLE_GPU=1` escape hatch. See [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

---

## 18. How to Use This Doc

When picking up a deferred item:
1. Move the relevant entry from this doc into the active implementation plan.
2. Update the "Phase Placement Key" at the top of this doc to mark the item as in-progress.
3. When the item ships, mark it as shipped and link to the doc section that describes the implementation.

If a deferred item's premise has changed (e.g., multi-feature concurrency becomes critical because the user base grows fast), promote it to the active plan and re-evaluate the placement.

---

## 19. Must an MCP write name a project listed on the grant?

**The question:** An MCP grant (`read` / `spend` / `configure`) is issued to a client through the consent screen. *Reads* span every Project — that was settled. Must a **write** that names a Project (`start_feature`, `start_ticket`, `apply_run_shape_patch`) name one that is **listed on the grant**, or may a `spend` / `configure` grant write to any Project? Under the assumption, `create_workspace_project` names no existing Project and would need a rule of its own — a grant cannot list a Project that does not exist yet.

**Never settled.** The design conversation covered reads spanning every project and did not decide the write side. The assumption carried in [`MCP_INTEGRATION.md`](MCP_INTEGRATION.md) is **writes name a project the grant lists**.

**Not built — nothing enforces the assumption.** `GrantRecord` has no project list, `oauth_grants` (migration V56) has no column for one, the guard and `POST /mcp` dispatch never compare a tool's `project_id` to anything on the grant, and neither `McpConsentView.tsx` nor `McpGrantsTab.tsx` mentions Projects. As built, a `configure` or `spend` grant can write to **any** Project. Treat the assumption as intent, not behaviour.

**Status: unresolved.** Settling it would need a decision record, a grant-schema migration, a consent-screen control and a guard check. Until then, do not describe the surface as project-scoped.

**Why open:** It was assumed rather than decided, so there is no recorded rationale for leaving it unenforced — only the fact that it is. Which way it lands is a user decision, not an implementation detail.

---

## 20. Should reviewing a fork's pull request run the fork's code?

**The question:** The code-review starter's second step, `s-validate-branch`, runs the Project's `prepare_command` and gate commands in a worktree checked out at the pull request's head. For a pull request opened from a fork, that head was written by someone outside the Project: a `postinstall` hook in its `package.json`, or a test file it adds, executes on the machine the Project runs on — the desktop, or its remote host — under the user's login shell and outside `PermissionPolicyPort`, before anyone has read the diff. The fix run started from a fork's pull request begins from the same code. Should Demeteo do that at all, and behind what consent?

**Never settled.** The first implementation shipped the behaviour without asking; a review caught it. What exists now is a stopgap chosen to keep fork reviews working, not an answer.

**What runs today (interim, provisional):**
- For a pull request flagged `from_fork`, the review launch shows a notice naming what will run and where, and "Review this PR" stays inert until the user ticks an acknowledgement. The tick covers one launch: it clears once that launch has been attempted, whether it started or failed, and selecting another pull request also clears it. So a later launch — including one after the fork pushed again — asks again.
- The confirm dialog of the fix run shows the same exposure for a fork's pull request.
- A pull request from the Project's own repository still reviews in one click, with no notice.

**The alternatives on the table:**
- **(b)** Skip the gate step for forks, and have the report say that no gate ran and why.
- **(c)** Put a new gate step with `gate_class: "dangerous"` ahead of `s-validate-branch`, so a human approves before anything executes — which also parks any unattended run at that point.
- **(d)** Ask for the acknowledgement on every pull request, forks or not.
- **(e)** A per-project "trust fork gates" setting that decides once instead of per launch.

(c) touches the workflow engine's step definitions and (e) touches `ProjectSettings`, so either is a larger change than the current stopgap, and adjacent to what AGENTS.md §6 already reserves for human approval.

**Status: open.** Closing it takes the user's choice among the interim behaviour and (b)–(e), and then an entry in [DECISIONS.md](DECISIONS.md) recording the rationale. Until then, do not describe the interim acknowledgement as the settled policy.

---

## 21. Should a gate this branch turned red still yield `branch-validation.md`?

**The question:** When a gate the branch under review broke goes red, `s-validate-branch` ends before its agent turn, so no `branch-validation.md` is written; the failing command and its output tail are kept as the step's recorded failure reason, and the fix run is seeded from that. Should the turn run anyway and write the report on a red branch too?

**What it would take:** a pure decision in `domain/verifier/` (possibly a per-step `StepConfig` flag, no migration), a change to the error arm of the agent step, and a pass of the two Docker conformance suites, which `npm run checks` does not run. It changes the harness-first contract, so it is a policy call rather than a fix.

**Status: open.** No one has asked for it yet; the recorded failure reason already names each red gate. It is written down so a later change to the review starter does not quietly decide it.

### 21a. Should the engine offer `environment` to a step that has nothing to configure?

**The question:** When a project configures no gate command, the verifier turn receives `HarnessOutcome::NotConfigured`'s `Harness Results — NOTHING RAN` section and the verdict contract, and both advise `environment` for exactly that case. So does a third source: when the turn ends without a verdict object, `read_step_verdict` resumes the session with `correction_prompt` (`steps/agent/verdict.rs`), whose menu advises `environment` for "something this project is not configured to run", and that re-ask is the last thing the model reads before it answers. On a review step that verdict maps to `Unjudgeable`: the step fails, its report is never recorded, and the review ends `failed`. The review starter's `s-validate-branch` judges nothing the project must configure, so `pass` is its right answer there, and today it gets it only by overriding both blocks by name in its own instructions and by saying, in advance, that a re-ask for the verdict alone still takes `pass`. The outcome rests on the model preferring the step-level sentences to three engine prompts, one of which arrives a turn after them.

**What closing it would take:** either no `environment` advice under `NotConfigured` for a step that declares nothing to configure, or a non-failing disposition for `environment` on a review step. Both are policy changes in `domain/` (`harness_outcome.rs`, `verifier/`), which is why the starter only overrides the advice in prose. Either must cover the correction re-ask too. A non-failing disposition does so by construction; withdrawing the advice does not, because the re-ask offers `environment` from its own menu, independent of the `NotConfigured` block, and a change that stops at the first turn fixes two sources of three and leaves the re-ask able to end the review `failed`.

**Status: open.** The test that renders the verifier prompt pins the override on the first turn only, where it cannot silently drift out of order. The re-ask is a separate turn that test never renders; a test beside `correction_prompt` pins that the re-ask still advises `environment` and that the starter still carries the sentence pre-empting it. Both are requests to the model, not a guarantee.

---

## 22. Should gate evidence reach the step that writes a published pull request body?

**The question:** A fix run started from a review is seeded with the review's findings and, below them, the gate evidence: the agent's `branch-validation.md`, or the gate step's recorded failure reason, which can quote the tail of the branch's own test and build output. All of it becomes the run's `feature_description`. The address-review starter ends in `s-finalize`, whose engine prompt interpolates that description and asks for a `pr_body` that is published on the remote. Gate output can carry home paths, hostnames and dumped environment variables. Should that text ever reach the finalize step?

**What runs today (interim):** two layers, neither of them a boundary.

- *The instruction (advisory).* The opening line of the gate fence tells the reader that the fenced text is for the run alone and must not be quoted into a commit message or the pull request's title or description. The finalize step reads the same description, so that line is the only route by which the instruction reaches it. Nothing enforces it.
- *The floor (mechanical).* Before the gate body is fenced, `sanitizedGateBody` (`src/lib/gateRedaction.ts`) redacts it and caps it, for both the report and the failure source. A private key block (`-----BEGIN … PRIVATE KEY-----` through its `END` line, or to the end of the body when the `END` line is missing) becomes one `[redacted private key]` line. Home directories become `~` (`/home/<name>`, `/Users/<name>`, `/root`, `<drive>:\Users\<name>`). These assignment values become `[redacted]`: an upper-case `NAME=value` line; an upper-case `NAME: value` or `"NAME": "value"` as YAML, JSON and Node's `console.log(process.env)` print it (a quoted value always, an unquoted one only when the name has a `_` and does not open with `ERR_`, so `ERROR: …`, `TS2345: …` and `ERR_MODULE_NOT_FOUND: …` lines stay whole); an upper-case `NAME_WITH_UNDERSCORE=value` inside a line, such as a command line; and any key, in any case, whose name contains `secret`, `password`, `passwd`, `token`, `api_key`, `access_key` or `private_key`, followed by `=` or `:`, unless its value is one complete backtick token, or its separator is `:` and its name's last `.`-segment holds no keyword and the value is a numeric source location, so `token.ts:12:5` stays a file and a line. A credential assignment inside a grep-style `path:line:content` hit is redacted. GitHub, GitLab, OpenAI/Anthropic, Slack and AWS access-key token shapes, `Bearer` tokens and `user:pass@` URL credentials also become `[redacted]`. RFC 1918 IPv4 addresses become `[redacted-ip]`. The body is then cut to its last 200 lines and 12,000 characters, behind one line that says how much was omitted. So an agent that quotes the fence anyway publishes the redacted excerpt, not the raw output.

The floor does **not** catch hostnames (internal or public), public IP addresses, IPv6, usernames outside a home-directory path, a home directory nested under another prefix (`/mnt/data/home/<name>`), a lower-case assignment whose name is not credential-shaped, a value quoted as one backtick token, or secrets in any shape it does not recognise: a bare password, a JWT outside a `Bearer` header, the body of a private key block whose `BEGIN` line was already cut off upstream, a vendor token with no fixed prefix. A fork's author controls the output and can print a secret in any of those shapes. Nor does the floor stop the gate body reaching the finalize prompt; it only changes what arrives there.

The review comment is the same concern handled the other way: `PostReviewComment` is handed the review report alone, and the gate evidence never reaches it.

**What closing it would take:** keeping gate evidence out of the finalize prompt — for example, passing it to the run's agent steps and not to `s-finalize` — which is a change to the engine's finalize prompt, not to the starter or the seed.

**Status: open.** The fence instruction and the redaction are mitigations, not a boundary. Until the engine change lands, do not describe gate evidence as kept out of what a fix run publishes.

---

## 23. Should a review whose gate step failed end the run `failed`?

**The question:** The code-review starter writes `code-review.md` in `s-review`, then runs the project's gates in `s-validate-branch`. On a red branch the gate step fails after the report is already written, so the engine ends the run `failed`. That is the case the review exists for, and the fix action reads that run's report and gate failure reason. The run did its job, yet every status surface says it failed. The gate step also fails on a harness timeout or a configuration error that ran no gate. So "failed" cannot simply be replaced with "gates red".

**What runs today (interim):** the feature detail view reads the run's step rows. When the review report was declared, the gate step failed with a recorded reason and no other step failed, its header chip reads **Review ready** in amber instead of **Failed** in ruby. `reviewEndedOnFailedGate` (`src/lib/reviewEvidence.ts`) decides this, and `REVIEW_READY_META` (`src/lib/runStatus.ts`) supplies the label. Only the label changes. The run's status stays `failed`, so retry, polling and every terminal-state check behave as they do for any failed run.

**The residual:** surfaces that hold a run's status string but not its step rows still show **Failed**. These are the pull-request list's review chip, the project home's run list and pipeline cards, ProjectTelemetry, and the Runs inbox. `REVIEW_READY_META` is deliberately not in `RUN_STATUSES`, because no run can persist it.

**What closing it would take:** an engine-side terminal status for a review that completed with its gate step failed. It would be persisted like `awaiting_mr` and would carry a label that every surface resolves through `runStatusMeta`. That touches the terminal-status set (`TERMINAL_STATUSES`, the engine's run finalisation) and every consumer that treats `failed` as the only unhappy ending. Once it lands, the detail-view predicate can go. A related issue sits on the same status model: a green review ends `awaiting_mr`, which reads "PR ready" on a workflow that cannot open a pull request.

**Status: open.** Until the engine change lands, do not describe a red review as distinguishable from a broken run anywhere except the feature detail header.
