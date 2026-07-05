# Epic A3 — Adapter conformance harness

> **Roadmap source:** [03-roadmap-6-months.md § Epic A3](../03-roadmap-6-months.md#epic-a3--adapter-conformance-harness); rank 4 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.1 (Sep)**.

**Outcome:** agent CLI churn becomes a failing CI check, not a user bug report; external contributors can add an agent from a doc.

**Epic acceptance:** all 5 working adapters covered; one simulated breaking change caught by CI in a drill.

**Why this matters now, not later:** the Gemini→Antigravity churn (Google killed Gemini CLI and replaced it inside one release cycle, see [market research §1](../01-market-research.md)) is the proof case. This must land *before* Epic C1's kanban board multiplies concurrent agent runs (see roadmap's "Deliberate sequencing couplings").

**Grounding facts (verified in repo, 2026-07-05):**
- The five adapters to cover: `claude_code`, `hermes`, `opencode`, `antigravity` (existing, `crates/demeteo-core/src/adapters/agent/{name}/mod.rs`) plus whichever of Codex/pi land first from Epics A1/A2. Each exposes a `runtime()` constructor consumed by `AgentRegistry` (`adapters/agent/registry.rs:13`), which is wired by hand in `crates/demeteo-core/src/composition/mod.rs:139-143`.
- Each adapter's `parse_event` function is the actual drift-detection surface — a CLI version bump that changes its JSON shape breaks parsing silently today (no test catches it). The harness's job is to make that a red CI check.
- No test infrastructure for this currently exists in the repo (confirmed no "board"/"kanban"-style prior art search was needed here since this is pure test/CI tooling, but there is also no existing golden-transcript fixture format to reuse — Epics A1/A2 will produce the first two fixtures organically; coordinate the fixture format with those stories rather than designing in isolation).

---

## Story A3.1 — Golden transcript corpus and replay tests

**As a** Demeteo maintainer, **I want** a recorded golden transcript per (agent, version) with a replay test that re-parses it, **so that** every adapter's `parse_event` logic is covered by a test that doesn't require the actual CLI binary to be installed in CI.

**References:**
- Architecture: `docs/ARCHITECTURE.md` § 3 Directory Layout (pick a test location consistent with existing test conventions in `crates/demeteo-core`).
- DDD Domain: `docs/DDD_MODEL.md` § 6 Agent Runtime.

**Status:** Not started.

**Tasks:**
- [ ] Define a fixture format: raw stdout/stdin bytes for one representative run per agent, tagged with the CLI version that produced it, plus the expected sequence of parsed `AgentEvent`s.
- [ ] Decide fixture location (e.g. `crates/demeteo-core/tests/fixtures/agent_transcripts/<agent>/<version>/`) — coordinate with whoever ships A1 (Codex) and A2 (pi) so their "record a golden transcript" tasks land in this format instead of inventing their own.
- [ ] Write a replay-test harness: for each fixture, feed the raw bytes through the adapter's real `parse_event` (or `SessionCliRuntime`'s read loop, for pi) and assert the resulting `AgentEvent` sequence matches the recorded expectation.
- [ ] Backfill fixtures for the four already-shipped adapters (`claude_code`, `hermes`, `opencode`, `antigravity`) — these don't exist yet and are needed to satisfy "all 5 working adapters covered."
- [ ] Wire the replay tests into the normal `cargo test` run (definition of done everywhere per the roadmap's operating rules: `cargo clippy` + tests clean).

## Story A3.2 — Nightly CI drift probe

**As a** maintainer, **I want** a nightly CI job that runs each installed agent CLI against a canned prompt and diffs its output shape against the recorded golden transcript, **so that** an upstream CLI update that breaks our parser opens an issue automatically instead of surfacing as a user bug report weeks later.

**Status:** Not started.

**Tasks:**
- [ ] Add a scheduled GitHub Actions workflow that installs each agent CLI at its latest version (using the install commands already catalogued in `docs/DECISIONS.md` decision 34), runs a minimal probe invocation, and compares the raw output shape (not full transcript — a structural probe: does it still parse into at least one valid `AgentEvent`, are required fields present) against the recorded fixture's shape.
- [ ] On drift, auto-open a GitHub issue tagged to the affected adapter with the diff attached.
- [ ] Include an `agy` (antigravity) headless-surface probe in this job specifically — this doubles as the "re-probe at each monthly release" mechanism the Epic A2 decision record and Epic A4 both depend on. Probe for: does `-p` mode now produce structured output, does `--print` still drop under non-TTY stdout, is there still no approval-policy granularity short of `--dangerously-skip-permissions`. Feed a positive result into the Epic A4 re-probe review, not a code change here.
- [ ] Run one deliberate drill: hand-edit a fixture to simulate a breaking upstream change, confirm the nightly job (or a manual trigger of it) actually fails red. This satisfies the epic's explicit acceptance criterion ("one simulated breaking change caught by CI in a drill") — do not skip this, it's the only acceptance test that proves the harness works rather than just existing.

## Story A3.3 — Contributor guide for adding an agent

**As an** external contributor who wants to add support for a new coding agent CLI, **I want** a doc that walks through the adapter interface without requiring deep codebase knowledge, **so that** agent breadth can grow from the community, not just from Demeteo's own roadmap.

**Status:** Not started.

**Tasks:**
- [ ] Write `docs/adapters/CONTRIBUTING-AN-AGENT.md` covering: the `AgentRuntime`/`AgentSession` port contract (`ports/agent_runtime.rs`), the choice between `UnifiedCliRuntime` (one-shot spawn) and `SessionCliRuntime` (persistent JSON-over-stdio, once A2 lands) and how to decide which fits a given CLI's headless surface, the `parse_event`/`build_args`/`perm_env` function-pointer pattern used by every existing adapter, how to register in `composition/mod.rs`, and how to add a golden-transcript fixture (Story A3.1's format).
- [ ] Include the Gemini→Antigravity lesson as a worked "why we require schema/session surfaces over scraping" rationale (market research §1's "strategic lesson" section) so contributors understand *why* the harness exists, not just how to satisfy it.
- [ ] Cross-link from the README's "Supported agents" section.
