# Demeteo Roadmap — August 2026 → January 2027

> **Purpose:** The actionable 6-month roadmap. Organized as **outcome themes**,
> not feature lists; sequenced as **Now / Next / Later** with monthly
> milestones. Derived from [market research](01-market-research.md) and
> [opportunity analysis](02-opportunities.md); sequencing rationale lives in the
> [high-level plan](04-high-level-plan.md).
>
> **Operating rules:**
> - Each epic has an *outcome statement* (the change in user behavior we're
>   buying), explicit *out-of-scope* cuts, and *acceptance criteria*. An epic
>   without all three doesn't enter a milestone.
> - This document is reviewed at the end of every month; "Later" items are
>   deliberately under-specified — do not detail them before their review gate.
> - Definition of done everywhere: `tsc --noEmit` + `cargo clippy` clean, app
>   boots without console errors, README/docs updated.

## The six themes

| Theme | Opportunity | Horizon |
|-------|-------------|---------|
| **A. Agent Coverage & Adapter Platform** | O1 | Now |
| **B. Demeteo Brain** (agents as the app's LLM) | O4 | Now |
| **C. Kanban & Task Orchestration** | O2 | Next |
| **D. Demeteo CLI** | O3 | Next → Later |
| **E. Memory v2** (OKF · pluggable engines · intelligence) | O5 | Later (format work starts Next) |
| **F. UX Quality & Trust** ([UX audit](../ux-audit/findings.md), Jul '26) | — | Now → Later (P1 → P2 → P3) |

> Theme F epics are named **UX1–UX3** (not F1–F3) to avoid colliding with the
> audit's finding ids `F1`–`F49`, which are referenced throughout.

Deliberate sequencing couplings:
- **B before C and E** — task generation (C2) and memory distillation (E)
  consume the BrainPort.
- **A's conformance harness before C** — a board multiplies concurrent agent
  runs; adapter reliability must precede that multiplication.
- **E's format (E1) before E's engine (E2)** — OKF files are useful with the
  current embedding backend; the Honcho adapter is worthless without a stable
  memory document model.
- **UX1 before C** — the board binds cards to pipelines through the
  start-feature surface; repo targeting that is collected-but-dropped (audit
  F2) and the always-firing conflict detector (F3) must be real before the
  board multiplies launches through them. Likewise UX2's composer/modal
  consolidation (F28) is C1 groundwork.

---

## NOW — Months 1–2 (August · September 2026)

### Epic A1 — Codex CLI adapter 🔴
**Outcome:** A Codex user downloads Demeteo and runs their first gated pipeline
with zero extra configuration.
**Scope:** New adapter under `crates/demeteo-core/src/adapters/agent/codex/`
using `codex exec --json` (JSONL event stream) mapped to `AgentEvent`;
availability probe via `AgentRegistry`; sandbox-mode passthrough config;
`CODEX_API_KEY`/ChatGPT-auth detection in preflight; cost extraction into the
existing pricing pipeline.
**Out of scope:** Codex cloud tasks, MCP config management, Windows-specific
sandbox tuning (track as follow-up).
**Acceptance:** golden-transcript test green; full feature → gate → merge run on
a real repo; README agent table updated.

### Epic A2 — pi coding agent adapter 🔴 *(pulled forward from Next)*
**Outcome:** together with Codex, provider coverage spans essentially the whole
model space — pi's unified LLM API drives Anthropic, OpenAI, Google/Gemini,
xAI and local models through one agent, making a native Antigravity adapter
unnecessary (see Antigravity decision below).
**Scope:** pi via `--rpc` (JSON-over-stdio session; strict LF-only framing) —
introduces `SessionCliRuntime` alongside `UnifiedCliRuntime`, the first
session-protocol adapter; availability probe; cost extraction.
**Out of scope:** long-session reuse across steps (follow-up in Next once the
session runtime has soaked).
**Acceptance:** golden-transcript test green; full feature → gate → merge run;
README agent table updated.

> **Antigravity decision (verified July 2026):** `agy`'s headless mode is
> query-only by default; tool execution requires `--dangerously-skip-permissions`
> (all-or-nothing), there is no documented structured output for `-p` mode, and
> `--print` is reported to drop stdout under non-TTY (pipes/subprocesses — our
> invocation path). That is below the adapter quality bar, so the "fix
> antigravity" backlog item is **de-scoped to a watch item**: (1) mark the
> adapter unsupported in the README honestly (done), (2) close
> `docs/OPEN_QUESTIONS.md` §17 with this decision record, (3) re-probe `agy`'s
> headless surface at each monthly release via the A3 harness; reinstate the
> adapter only when structured output + a usable approval policy exist. See
> [market research §1](01-market-research.md) for sources.

### Epic A3 — Adapter conformance harness
**Outcome:** agent CLI churn becomes a failing CI check, not a user bug report;
external contributors can add an agent from a doc.
**Scope:** Golden transcript corpus per (agent, version); replay tests for every
adapter (including the shipped three); nightly CI job that probes installed
agent versions against recorded ones and opens an issue on drift; an `agy`
headless-surface probe for the Antigravity watch item; a
`docs/adapters/CONTRIBUTING-AN-AGENT.md` guide.
**Acceptance:** all 5 working adapters covered; one simulated breaking change
caught by CI in a drill.

### Epic B1 — BrainPort + generated titles & PR descriptions
**Outcome:** every pipeline gets a meaningful title and every MR a real
description, with no new configuration.
**Scope:** New `BrainPort` in `demeteo-core/src/ports/`; default adapter
invokes the user's preferred *already-configured* coding agent one-shot
(Codex `--output-schema` where available; print/JSON mode otherwise) with typed
output schemas; wire into pipeline creation (title) and `MrPublisher`
(description from diff + step summaries); per-call token/cost accounting into
the existing cost view; graceful fallback to current templates when no agent is
available; kill-switch in Preferences.
**Out of scope:** step-level summaries, commit messages, gate summaries
(B2, Next); using the Memory Agent's LLM config (that config is being *retired
into* BrainPort in E1, not extended).
**Acceptance:** PR descriptions generated on 100% of publishes when an agent is
present; measured added latency < 15s p90 per publish.

### Epic UX1 — High-severity UX defect burn-down 🔴
**Outcome:** the app stops misleading users on core journeys — every P1
finding from the [July 2026 UX audit](../ux-audit/findings.md) is closed, and
the review half of the core journey (code browsing, artifacts, gates) works
offline. This is the UX half of the v1.1 credibility story: an agent table
that's honest *and* an app that does what its buttons say.
**Scope (audit F1–F6, F34–F36):**
- "Stop Step" no longer cancels the whole feature — rename or implement a real
  per-step stop (F1); wire `target_repos` through `start_feature` or demote
  the pickers to informational chips (F2); per-feature repo association so the
  launch-modal conflict detector stops flagging every repo (F3) — both
  prerequisites for the board's card→pipeline binding (C1).
- Bundle in-app docs via `?raw` imports + `react-markdown` (already a runtime
  dependency) so help works in packaged builds (F4); collapse the three
  keyboard-shortcut sources of truth into the registry and mount the dead
  `ShortcutHelp` overlay (F5).
- Idempotent bootstrap retry — no duplicate project rows (F6).
- Bundle Monaco (`monaco-editor` dep + `loader.config`) so Browse Code,
  artifact previews, and gate review work offline and stop phoning a CDN (F34).
- One overlay/Escape priority stack consulted by Escape, `Cmd+W`, and mouse
  back/forward — closing a modal must never also navigate the view underneath
  (F35, F40).
- Delete the ~3,200 lines of dead parallel implementations (second
  create-from-zero wizard, unused overlays) (F36).
**Out of scope:** P2/P3 findings (UX2/UX3); any new capability beyond what the
fixes require.
**Acceptance:** all nine P1 findings closed with regression coverage; offline
smoke test passes (docs panel, Browse Code, gate artifact review with network
disabled); Escape/mouse-back never navigates beneath an open overlay.

### Milestone M1/M2 exit criteria
Release **v1.1**: Codex + pi + harness + Brain-generated titles/descriptions;
Antigravity honestly de-scoped; **zero open P1 UX-audit findings**. Announce
with a "5 agents, every model provider, governed pipelines" story
(claude-code, opencode, hermes, Codex, pi).

---

## NEXT — Months 3–4 (October · November 2026)

### Epic A4 — MiniMax adapter + pi session reuse
**Outcome:** breadth story reaches 6 working agents; session runtime matures.
**Scope:** MiniMax via Mini-Agent/M2 non-interactive mode (MMX-CLI is **not**
an agent — optionally expose as a step tool later); pi long-session reuse
across steps on the `SessionCliRuntime` shipped in A2; Antigravity re-probe
review (reinstate only if the watch criteria in A2's decision record are met).
**Acceptance:** MiniMax in the conformance harness; pi session reuse
demonstrated across a multi-step workflow.

### Epic C1 — Kanban board MVP (per project)
**Outcome:** work *originates* in Demeteo: users plan in a board and execute
cards as pipelines, instead of pasting one feature at a time.
**Scope:** Board with columns (Backlog / Ready / Running / In Review / Done);
card = title, description, labels, **assigned workflow version + gate policy**;
"Run" on a card starts the existing feature pipeline and binds card state to
pipeline state (Running/In Review track execution and gates automatically);
card queue honoring the current serial-execution limit; SQLite schema +
repository following existing DDD layout.
**Out of scope (cut ruthlessly):** swimlanes, sprints, estimates, multi-user
sync, external Jira/Linear import, cross-project boards, board automation
rules.
**Acceptance:** full journey — create cards, assign workflow, run, gate,
merge — without leaving the board; UX journey doc added to `UX_JOURNEYS.md`.

### Epic C2 — AI task generation (Brain-powered)
**Outcome:** "describe a goal → get a reviewed set of board cards."
**Scope:** BrainPort call that decomposes a plain-language goal into proposed
cards (each with suggested workflow from the project's existing workflows);
human review/edit step before cards land in Backlog (proposals are a gate, in
keeping with the product's philosophy).
**Acceptance:** proposal→accept flow shipped; ≥70% of generated cards accepted
without edits in dogfooding on this repo.

### Epic D1 — `demeteo` CLI: read + trigger
**Outcome:** terminal users script Demeteo; the app is no longer the only door.
**Scope:** New `crates/demeteo-cli` (second driving adapter over
`demeteo-core` application services — no logic in the CLI crate); commands:
`demeteo projects list`, `demeteo board list/add`, `demeteo run <card|feature>`,
`demeteo status --watch`, `demeteo gates list/approve/reject`; `--json` on every
command; shares the app's SQLite + config (single-writer discipline documented
in `DECISIONS.md`).
**Out of scope:** daemon mode, CI service accounts, remote (network) control of
a running app, package-manager distribution (Later).
**Acceptance:** an overnight script can queue 3 cards, run them serially, and
leave gates pending for morning review — the tier-3 story, demonstrated.

### Epic E1 — Memory v2 groundwork: OKF format + MemoryBackendPort
**Outcome:** project memory becomes a human-readable, git-versionable OKF
directory; engines become swappable.
**Scope:** Serialize existing typed memories (conventions/lessons/decisions/
preferences/facts) to an OKF v0.1 directory (Markdown + YAML frontmatter) at a
per-project location (default in-repo `.demeteo/memory/`, configurable);
two-way sync with current store; define `MemoryBackendPort` (ingest signal,
distill, recall-for-context) and move the existing embeddings implementation
behind it; retire the Memory Agent's separate LLM config in favor of BrainPort
for distillation (embeddings endpoint config remains).
**Out of scope:** Honcho, cross-project features, suggestion engine (E2/E3).
**Acceptance:** memories visible/editable as files and in-app with edits
surviving round-trip; a coding agent in a pipeline step can read the OKF
directory directly; migration for existing users is automatic and reversible.

### Epic UX2 — Truthful UI & surfaced capability
**Outcome:** what the UI shows matches what the backend does — no fabricated
telemetry, no silently dropped input, no silently swallowed errors, and no
registered backend command without either a UI surface or a recorded decision
that it stays headless. Closes the [UX audit](../ux-audit/findings.md)'s P2
tier.
**Scope (audit P2 cluster, F7–F24 · F37–F47):**
- *Truthful state & copy:* real status chips in the Project Home pipeline list
  instead of everything reading "RUNNING FLEET" (F9); real provider/workflow
  header copy (F10); correct data paths in About (F11); remove the fabricated
  "nodes" metric (F42); one consistent agent-kind list — `antigravity` banned
  everywhere while de-scoped (F21, ties to the A2 decision record);
  empty-state tiles renamed to what they do (F22); wizard-created projects
  show the project name, not the feature title (F41).
- *Silent failures become visible:* error surfacing on every save path (F29,
  F45); "Test Connection" stops silently creating machines (F37);
  WorkflowEditor dirty-state guard so Back/Escape can't discard prompt
  templates (F38); remote launches warn about (or disable) attachments,
  per-step overrides, and commit-artifacts they currently drop (F13).
- *Surface built capability* (registered commands with no UI): pause/resume
  buttons and workflow import (F12); a `conflict_detected` listener wired to
  the notification bell (F24); post-sync re-validation instead of telling the
  user to do it manually (F44); post-launch attachments via the existing
  direct-mode dropzone (F47); notification click-through navigation (F26).
- *Navigation correctness:* back-stack dedup for unmatched view kinds (F14);
  `Cmd+G` cycling scoped to the current project (F16); new-project state
  carries `compute_type`/`remote_host` (F7).
- *C1 groundwork:* consolidate the start-feature composer/modal pair and the
  triplicated strategy form into single components (F28) — the board reuses
  these surfaces, so consolidation must precede it, not follow it.
**Out of scope:** P3 polish (UX3); visual redesign.
**Acceptance:** audit P2 list closed, or per-item waived at M3 review with a
decision record; every registered Tauri command has a UI surface or a
documented headless-only decision; provider alias persists and edit means
update, not re-connect (F8).

### Milestone M3/M4 exit criteria
Release **v1.2**: board MVP + task generation + CLI (read/trigger) + OKF
memory + 6 working agents; **UX-audit P2 tier closed or waived with decision
records**. This is the "plan → run → learn, from app or terminal"
release — the positioning release of the half.

---

## LATER — Months 5–6 (December 2026 · January 2027)

> Scoped at the M4 review; items below are direction + guardrails, not specs.

### Epic E2 — Honcho engine adapter (opt-in)
Second `MemoryBackendPort` adapter over Honcho's HTTP API (self-host or cloud).
**Hard constraints:** out-of-process only (AGPL boundary — record in
`DECISIONS.md`); feature-gated opt-in with the local embeddings default
untouched; map project/agents/user to Honcho peers; use the Dialectic API for
recall. Kill criterion at M4 review: if E1 telemetry shows recall quality is
not the binding constraint, defer E2 in favor of E3.

### Epic E3 — Memory intelligence ("second brain" features, first slice)
Cross-project relations (shared conventions/patterns detected across a user's
projects, stored as OKF links); **context packs** — per-step context assembled
from memory + board history; a "suggested next features" panel on Project Home
fed by memory + completed-card history (suggestions are proposals → C2 flow).
One slice ships; the rest is the 2027 H1 headline.

### Epic D2 — CLI maturity
`demeteo init` (bootstrap a project from a repo path), non-interactive gate
policies for unattended runs (auto-approve rules with audit log), Homebrew/npm
distribution, CI recipe docs (GitHub Actions example draining a board column).

### Epic C3 — Board × concurrency
Lift strict serial execution to per-project `max_concurrent_features` with
board WIP limits as the control surface (resolves `docs/OPEN_QUESTIONS.md` §1);
per-project LLM spend budget wired to the existing cost tracking.

### Epic A5 — Agent watch-list review
Re-score Amp / Cursor Agent / Crush / Qwen Code / Aider / Droid / Goose against
user requests and conformance-harness cost; commit at most two.

### Epic UX3 — Consistency system & polish
The [UX audit](../ux-audit/findings.md)'s P3 tier plus its cheap-win
opportunities, scoped precisely at the M4 review. Direction:
- One status-color vocabulary via the existing `StatusBadge` (F27); one styled
  confirmation primitive replacing the `window.confirm` / Tauri-dialog /
  custom-modal mix, with confirmations added where absent (F23, F49).
- Dedupe utility helpers (`formatTokens`, `fuzzyMatch`, relative-time) (F28);
  CodeEditorView/terminal papercuts (F48); rail overflow beyond 8 projects
  (F32); the F33 misc list.
- Audit opportunities that compound with this half's themes: features and
  workflows searchable from the command palette (makes `Cmd+K` the universal
  switcher the board era needs); persisted UI state via the existing
  `get/set_app_session` commands; a cost column in the feature list.
**Guardrail acceptance:** one status mapping, one confirm primitive, UI state
survives restart.

### Milestone M5/M6 exit criteria
Release **v1.3** by end of January 2027. Candidate headline: "the orchestrator
with a memory" — Honcho or intelligence slice, whichever survived the M4
review, plus unattended CLI operation and the UX3 consistency guardrails.

---

## Success metrics (track monthly, review at each milestone)

| Metric | Baseline (Jul '26) | Target (Jan '27) |
|--------|--------------------|------------------|
| Working agent adapters | 3 (1 broken) | 6+, zero broken claims in README |
| Adapter regressions reaching users | n/a | 0 (caught by harness) |
| % pipelines with generated title + PR description | 0% | >90% (agent present) |
| Features initiated from board vs. modal | 0% | >50% of dogfood usage |
| CLI-initiated runs | 0 | >20% of dogfood runs |
| Projects with OKF memory dir committed to repo | 0 | dogfood + early adopters |
| Open UX-audit findings (P1 / P2) | 9 / 29 | 0 / 0 (or waived w/ decision record) |
| Backend commands with no UI surface & no decision record | 6+ (pause/resume, import, …) | 0 |
| Release cadence | ad hoc | monthly minor release |

## Risk register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Agent CLI breaking changes mid-integration | High | Medium | A3 harness first-class; prefer schema/session surfaces |
| Kanban scope creep into full PM suite | High | High | C1 out-of-scope list is contractual; monthly review enforces cuts |
| Brain latency degrades UX | Medium | Medium | Async generation with template fallback; p90 budget in acceptance |
| AGPL contamination via Honcho | Low | High | Out-of-process only; decision record; no Honcho code vendored |
| OKF spec churn (v0.1 → v1) | Medium | Low | It's Markdown+frontmatter; version-tag our dirs; migration is textual |
| SQLite contention app↔CLI | Medium | Medium | Single-writer discipline + WAL; documented in DECISIONS.md |
| UX-debt fixes crowd out feature epics | Medium | Medium | P1s are contractual in v1.1 (credibility story); P2s waivable per-item at M3 with decision record; P3 scoped only at M4 |
| New surfaces (board, CLI) re-introduce audit-class defects | Medium | Medium | UX1's overlay/Escape stack and F28 consolidation land *before* C1 builds on those surfaces; audit checklist applied to C1 UX review |
| Solo/small-team bandwidth vs. 6 themes | High | High | Now/Next/Later gates; each milestone shippable alone; A5/E2 have explicit kill criteria; UX2/UX3 items individually waivable |

## Explicitly not doing (this half)

Multi-user / team sync and RBAC · Jira/Linear/GitHub-Projects import ·
cloud-hosted Demeteo service · agent marketplace · mobile/web app ·
building our own foundation-model integrations beyond BrainPort ·
MMX-CLI as an agent (tool candidate only) · Windows-Intel-mac packaging work
beyond current CI.

## Review cadence

- **Monthly:** milestone review — demo against acceptance criteria, re-score
  "Next" items, update this doc in the same PR as the release notes.
- **M2 (end Sep):** go/no-go on C1 scope; first Antigravity watch-item re-probe.
- **M3 (end Oct):** UX2 waiver review — any audit P2 finding not shipping in
  v1.2 gets a per-item decision record.
- **M4 (end Nov):** E2 vs E3 priority call (kill criterion above); C3 go/no-go;
  UX3 scope selection from the audit's P3 tier + opportunity list.
- Any agent-market shock (new major CLI, another Gemini-style sunset) triggers
  an out-of-band re-plan of Theme A only — other themes hold.
