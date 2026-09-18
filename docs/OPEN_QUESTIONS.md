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

**The answer now:** The visual DAG builder (`PRD_DAG_WORKFLOWS.md`) supersedes the form editor entirely, and [decision 42](DECISIONS.md#1-the-42-decisions) ships a **read-only Monaco JSON source tab** in builder Phase 3. See the superseded entry for [decision 19](DECISIONS.md#2-superseded-decisions).

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

**The v1.0 answer:** *Partial.* Per-project defaults live on `ProjectSettings::default_agent_kind` and `default_model`. Per-workflow overrides live on `ProjectWorkflowOverride` rows (`step_id = None` for workflow-level, `Some(step_id)` for step-level). Per-feature overrides are snapshotted onto the feature row (`Feature::agent_kind`, `Feature::model`, `Feature::step_overrides`). A `WorkingMemoryEntry` shape exists for thread-scoped working memory. The per-machine `AgentProfile` rows exist for the legacy shell / custom-http agent kinds (ollama, openai, cli, custom_http) and are still managed in `Machine` / `AgentProfile`. Demeteo does **not** read, store, or inject the LLM API key itself; the user configures the agent on the host (decision 4).

**The deferred work:** A first-class structured `AgentConfig { kind, model, workdir, env_refs, model_pricing_override }` per machine, editable from `EnvModal` (or its successor). Per-step override of the default beyond the current `StepOverride` snapshot.

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
  A *chat channel* for clearing a parked Gate is a narrower thing and does not reopen
  this: see [REMOTE_GATE_APPROVAL.md](REMOTE_GATE_APPROVAL.md), in discovery.
- **WebKitGTK + NVIDIA + Wayland Error 71** — not a deferred feature, but a documented platform quirk with an auto-detected workaround and `DEMETEO_DISABLE_GPU=1` escape hatch. See [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

---

## 18. How to Use This Doc

When picking up a deferred item:
1. Move the relevant entry from this doc into the active implementation plan.
2. Update the "Phase Placement Key" at the top of this doc to mark the item as in-progress.
3. When the item ships, mark it as shipped and link to the doc section that describes the implementation.

If a deferred item's premise has changed (e.g., multi-feature concurrency becomes critical because the user base grows fast), promote it to the active plan and re-evaluate the placement.