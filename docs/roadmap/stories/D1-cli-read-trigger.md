# Epic D1 — `demeteo` CLI: read + trigger

> **Roadmap source:** [03-roadmap-6-months.md § Epic D1](../03-roadmap-6-months.md#epic-d1--demeteo-cli-read--trigger); rank 8 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.2 (Nov)**.

**Outcome:** terminal users script Demeteo; the app is no longer the only door.

**Out of scope:** daemon mode, CI service accounts, remote (network) control of a running app, package-manager distribution (all deferred to Epic D2, Later).

**Epic acceptance:** an overnight script can queue 3 cards, run them serially, and leave gates pending for morning review — the tier-3 "overnight backlog draining" story, demonstrated end-to-end.

**Grounding facts (verified in repo, 2026-07-05):** **Greenfield.** The workspace's `Cargo.toml` currently lists exactly two crate members: `crates/demeteo-core` and `crates/demeteo-runner` (plus `src-tauri`). `crates/demeteo-cli` does not exist — this epic adds a new workspace member from scratch.

**Dependency:** Epic C1 (kanban board) should exist before `board list/add`/`run <card>` commands have anything real to call — check C1's status before starting Story D1.3.

---

## Story D1.1 — Scaffold `demeteo-cli` as a second driving adapter

**As a** Demeteo maintainer, **I want** a new CLI crate that calls `demeteo-core`'s application services directly, **so that** the CLI is a thin driving adapter over the same hexagon the Tauri app already uses — not a second implementation of any business logic.

**References:**
- Architecture: `docs/ARCHITECTURE.md` § 1 The Hexagon — `demeteo-core` is already isolated from the Tauri shell; per the roadmap, "a CLI is a second driving adapter over the same application services, not a rewrite." Study how `src-tauri` currently constructs and calls into `demeteo-core`'s composition root (`crates/demeteo-core/src/composition/mod.rs`) as the pattern to mirror for the CLI's own thin entry point.

**Status:** Not started.

**Tasks:**
- [ ] Add `crates/demeteo-cli` as a new workspace member in the root `Cargo.toml`.
- [ ] Wire its `main.rs` to construct the same composition root (`demeteo_core::composition`) the Tauri app uses, so both binaries share identical application-service wiring — no CLI-specific business logic, only argument parsing + output formatting.
- [ ] Pick a CLI argument-parsing crate (check if `clap` or similar is already a dependency anywhere in the workspace before adding a new one).
- [ ] Confirm the CLI and the desktop app share the same SQLite database file location and config — this needs the single-writer discipline addressed in Story D1.4 before both can safely run concurrently.

## Story D1.2 — Commands: projects, board, run, status, gates

**As a** terminal user, **I want** `demeteo` subcommands covering the read/trigger surface, **so that** I can inspect and drive Demeteo without opening the desktop app.

**Status:** Not started.

**Tasks:**
- [ ] `demeteo projects list` — list projects, reusing `ProjectRepository` directly (no new query logic).
- [ ] `demeteo board list` / `demeteo board add` — list/add cards on a project's board; **depends on Epic C1's `Card`/`Board` domain model existing** — if C1 hasn't shipped yet when this story starts, stub this command with a clear "not yet available" message rather than blocking the whole CLI on it.
- [ ] `demeteo run <card|feature>` — trigger a run, calling the same `start_feature` path (or card "Run" path from Epic C1's Story C1.4) the UI uses.
- [ ] `demeteo status --watch` — poll and print status of active features/cards; `--watch` mode should poll on an interval and print diffs, not spam full state every tick.
- [ ] `demeteo gates list` / `demeteo gates approve` / `demeteo gates reject` — list pending gates and resolve them, calling the same `gate_decide` path `GateView` uses (`docs/ARCHITECTURE.md` § 4, `GatePresenter::gate_decide`).

## Story D1.3 — `--json` on every command

**As a** user scripting Demeteo, **I want** every command to support `--json` output, **so that** I can pipe output into `jq` or other tooling instead of parsing human-readable text.

**Status:** Not started.

**Tasks:**
- [ ] Add a global `--json` flag that switches every command's output format from human-readable to structured JSON.
- [ ] Define stable JSON shapes for each command's output (don't just `serde_json::to_string` internal domain structs directly if their shape is likely to change — consider a thin DTO layer, but don't over-engineer this for a v1 CLI with 5-ish commands).

## Story D1.4 — Shared SQLite + single-writer discipline

**As a** user running the CLI alongside the desktop app, **I want** both to safely share one database, **so that** I don't corrupt state by running them concurrently.

**References:** `docs/DECISIONS.md` — this needs a new decision entry documenting the single-writer discipline, per the roadmap's explicit instruction ("single-writer discipline documented in `DECISIONS.md`").

**Status:** Not started.

**Tasks:**
- [ ] Confirm SQLite's WAL mode is enabled (check existing DB setup in `demeteo-core`) — WAL allows concurrent readers with one writer, which is the mechanism this story should rely on rather than building a custom lock.
- [ ] Verify/enforce that only one process writes at a time in practice — decide whether this needs an actual file lock (e.g. an advisory lock alongside the DB file) or whether WAL + short transactions is sufficient given the CLI's usage pattern (short-lived commands, not a long-running daemon — daemon mode is explicitly out of scope for this epic).
- [ ] Document the discipline in `docs/DECISIONS.md` as a new numbered decision (the existing table runs 1–36; this would be decision 37 — confirm the current max number before numbering, since other epics in this same roadmap cycle may add decisions too).
- [ ] Add a test or manual verification: run a CLI command and a desktop-app action against the same DB in quick succession, confirm no corruption/lock errors surface to the user.

## Story D1.5 — Overnight tier-3 acceptance walkthrough

**As a** maintainer validating this epic, **I want** to actually run the "queue 3 cards overnight, run serially, leave gates pending for morning" story, **so that** the epic's acceptance criterion is demonstrated, not assumed.

**Status:** Not started.

**Tasks:**
- [ ] Write a script using `demeteo board add` (×3) + `demeteo run` to queue and kick off three cards.
- [ ] Let them run serially overnight (respecting the existing strict serial-execution limit, `docs/OPEN_QUESTIONS.md` §1 — this is not new concurrency, just CLI-driven use of the existing queue).
- [ ] Confirm `demeteo gates list` the next morning shows the expected pending gates, and `demeteo gates approve` resolves them correctly.
- [ ] Document this walkthrough (command sequence + expected output) somewhere durable — either in this epic's story file or in a new `docs/` CLI usage doc — so it doubles as a regression script for future releases.
