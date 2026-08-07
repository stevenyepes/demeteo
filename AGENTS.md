# Demeteo — Agent Constitution

> Fleet-style multi-agent orchestrator: Tauri v2 (Rust) + React 19 (TypeScript).
> This file carries only what is **not** discoverable by reading the code —
> invariants, Gate policy, the commit contract, the verification gate. Anything
> structural lives in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md); §8 maps the rest.

Scripts are in `package.json`; run `npm run` to list them.

---

## 1. Project Identity

**Demeteo** lets a developer describe a feature in plain language; the app decomposes
it into a Workflow, delegates Steps to coding agents (opencode, claude-code, hermes),
manages a Git worktree per Step, and presents human-approval Gates before merging.
**Current phase: V1** — core orchestrator, fully implemented.

Use these exact names in code and comments: **Project** (a Git repo Demeteo tracks) ·
**Feature** (user-described work, decomposed by a Workflow) · **Workflow** (reusable,
versioned DAG of Steps) · **Step** (one DAG node: `agent`, `parallel`, or `gate`) ·
**Subtask** (work for one agent in one worktree) · **Gate** (human-approval checkpoint) ·
**ProviderInstance** (a configured AI provider: model + key + endpoint).

---

## 2. Architectural Invariants

Decisions, not descriptions — several are about what deliberately *isn't* here, which
leaves no trace in the code to discover. **None of these have an approved workaround.**
If a task appears to require violating one, stop and say so rather than asking to
proceed; that distinguishes them from the Gate items in §6.

- Agent integration is **one-shot CLI + JSON only** — no ACP, no JSON-RPC, no tool-call bridge
- **`ExecutionPort` is the one behavioural contract every transport satisfies identically** — local subprocess, desktop-over-SSH, and `demeteo-runner`. A feature must behave and render the same regardless of which one ran it. Never branch on transport in calling code: if the transports differ, the adapter or the contract is wrong, not the caller. The contract is specified in the trait's own rustdoc (`crates/demeteo-core/src/ports/execution.rs`) → [docs/EXECUTION_PARITY.md](docs/EXECUTION_PARITY.md)
- The compiled `PermissionProfile` is **complete** and uses only `allow` / `deny`, never `ask` — there is no real-time human-in-the-loop at the tool level
- Never bypass `PermissionPolicyPort` when spawning agent processes
- Agents are fenced to their worktree by `external_directory: "deny"` plus the OS-level chmod fence in `adapters/worktree/git_ops/scope.rs` — never widen it
- Secrets live in the OS keyring only — never write credentials, tokens, or secrets to SQLite or any file
- `demeteo-runner` ships as a Linux x86_64 musl static binary — build it with `npm run build:runner`, never a bare `cargo build` → [docs/RUNNER_DEV.md](docs/RUNNER_DEV.md)
- Never mutate a harness's own persisted config (`~/.codex/config.toml`, `$HERMES_HOME/config.yaml`, `~/.claude`, shell rc files). What Demeteo tells a harness is **per-invocation** — CLI flags or child-process env, never written to disk. Run shape persists only in Demeteo's own SQLite (`ProjectSettings.default_effort`, `Feature.effort`). If a harness exposes a setting *only* through its config file (as Hermes does for reasoning effort), declare the capability unsupported and degrade honestly rather than reaching into that file.

---

## 3. Code Conventions

Match the surrounding code for anything not listed here — naming, file layout, export
style, and formatting are evident from neighbouring files. These are only what a reader
of one file would *not* infer:

- Don't break the hexagon — no business logic in `commands/`, no adapters called from the frontend
- All Tauri commands called through typed wrappers in `src/lib/` — never `invoke()` raw in a component
- No `any` — use `unknown` + a type guard when the shape is uncertain
- One component per file; extract when a file passes ~400 LOC
- `#[tauri::command]` returns `Result<T, String>` — map with `.map_err(|e| e.to_string())`
- `thiserror` for domain error enums in `src-tauri/src/domain/`
- All DB access through `src-tauri/src/db.rs` — no raw `rusqlite` in commands
- Never `.unwrap()` / `.expect()` in production paths — use `?` or match
- Never hard-code `localhost`, port numbers, or paths — read them from config/state

### Where a decision is allowed to live

The frontend's ~400 LOC rule has no Rust counterpart, which is how
`steps/sequence/mod.rs` reached 1818 lines without ever tripping a review
reflex. Line count was the symptom; this is the cause:

- **A policy decision must not be spelled inside an `async fn` that also does
  I/O.** If a `match` decides *what should happen* — as opposed to performing
  it — it belongs in `domain/`, where it is synchronous and reachable from a
  test without a single port double. `domain/` has no `async fn` anywhere in
  it; keep it that way and the boundary enforces itself.
- **Never construct an `ExecutionDriver` in a test.** It carries twenty-odd
  ports that the code under test does not read (`driver_watchdog.rs` has two
  `#[ignore]`d tests conceding exactly this). When adapter code is unreachable
  from a test, the fix is to make it a free function over the *one* port it
  needs — not to stub the other nineteen.
- Extract a stage when an adapter module passes ~400 LOC **of code**. Doc
  comments do not count toward it; they are the part worth keeping.
- `#[allow(clippy::too_many_arguments)]` is a review trigger, not a fix. Bundle
  the parameters that travel together, then delete the attribute.

### Comments

**Default to no comment.** The generative reflex is to narrate, and narration is
the one kind of comment this tree does not have: a sweep of `demeteo-core` found
16 narration openers across 60k lines, against 12k lines of rustdoc that carry
decisions. Holding that ratio is the point — every comment you add is an
exception that has to earn its place, and most do not.

A comment earns it by carrying what is **not** discoverable by reading the code
— the rule this document applies to itself. An agent recovers *what* cheaply and
cannot recover *why* at all, so the measure is non-recoverable information, never
length. A thirty-line header recording a decision earns its place; `// increment
the counter` does not, at any length. Never cap or trim a comment by size alone.

**Do not write these** — each is a review trigger on its own:

- Restatement: derivable from the line under it, or rustdoc re-spelling the
  signature (`/// Returns the name.` over `fn name()`).
- Change narration — `// Now also handles the remote case`, `// Fixed: used to
  return early`. That describes a diff, not the code: stale on landing, and it
  reads as authoritative forever. The commit message holds it instead.
- Ticket echoes, and play-by-play (`// Step 1`, `// Now build the args`). If the
  steps need labelling they need naming — extract functions.
- Explaining Rust or the stdlib to the next reader.
- `TODO`/`FIXME` as an invitation: an agent reads it as an instruction and acts
  on it mid-task. Leave an inert fact with a scope, or a ticket.

**The traps:**

- **Prefer encoding over describing.** A type or test that makes the wrong thing
  impossible beats a comment asking for the right thing — a comment can lie, a
  test cannot. `domain/` having no `async fn` is why that boundary holds; a
  comment requesting it would not.
- **Put a constraint where the wrong edit would be made**, not only in `docs/`.
  Agents read slices: the doc may be out of context, the line above the
  temptation never is. "We tried X, it breaks Y" is worth more than it looks,
  because agents rewrite unfamiliar code a human would leave alone.
- **You changed the code, you own the comment above it.** A stale comment is
  worse than none — adjacent prose reads as high-confidence signal and steers
  the next agent wrong. Never cite line numbers; the tree has exactly one.
- If a diff's comment density is visibly above the file around it, you narrated.

### Cross-OS

The desktop app ships on **Linux x86_64, macOS aarch64, and Windows x86_64**
(the `build.yml` matrix). The remote runner is always Linux. So: host-side code
must work on all three; only remote-side code may assume Linux and systemd.

PR checks run on `ubuntu-22.04` only — **green locally and in CI does not prove
macOS or Windows even compiles.** That breakage surfaces on master or a tag,
after merge. When you touch host-side paths, shells, or process handling, reason
about all three targets before you finish, and say which ones you couldn't verify.

- Build paths with `Path`/`PathBuf::join` — never string-concatenate separators
- Resolve data/config/cache dirs through the platform API — never hard-code `~/.local/share`, `$TMPDIR`, or `/tmp`
- Don't assume a POSIX shell host-side; put shell invocation behind `cfg`
- macOS filesystems are case-insensitive by default — a wrong-case import compiles on a Mac and fails on Linux CI

---

## 4. Visual Design

Token *values* live in `src/App.css` — read them from there, never hard-code hex.
Their semantics, which the stylesheet doesn't record: **violet** = active connections
and primary actions · **cyan** = terminal streams and interactive states · **emerald** =
running agents and healthy statuses · **ruby** = errors, stopped tasks, failures.

Cards are glassmorphism — `backdrop-filter: blur(12px)` over the card-surface token.
Headings `Outfit`, UI `Inter`, terminal/code `Fira Code` / `JetBrains Mono`. Status dots
pulse; view switches transition smoothly. **Never** plain system colors, `style=` props
for design tokens, or flat grey cards with no depth.

---

## 5. Commit Convention

[Conventional Commits 1.0.0](https://www.conventionalcommits.org/), enforced by the
`commit-msg` hook and the `Lint Commits` workflow. Release automation infers the semver
bump from the type, so the type is load-bearing.

```
<type>(<scope>): <subject>     # ≤72 chars, imperative, no trailing period
```

Types: `feat fix perf revert refactor docs style test build ci chore`. `feat` → minor,
`fix`/`perf`/`revert` → patch, rest → no bump; `!` or a `BREAKING CHANGE:` footer → major.

**The trap:** `subject-case` rejects a subject starting with a capitalized token — a
ticket id, acronym, or `TypeName`. `feat(remote): P0 multi-client runner` fails;
`feat(remote): multi-client runner P0` passes.

Verify before committing: `echo "<message>" | npx commitlint`

---

## 6. Gate Policy (Human Approval)

Stop and ask the user before doing any of these. Unlike §2 these are permitted —
they just aren't yours to decide alone.

- Adding an `npm` or `cargo` dependency
- Any migration that deletes or renames an existing file in `crates/demeteo-core/migrations/`, or that drops or renames a column
- Changing `src-tauri/capabilities/` (Tauri permission surfaces)
- Changing agent spawn logic or `OPENCODE_PERMISSION` env construction
- Merging worktrees back to a feature branch when conflicts are detected
- Re-running `Promote Release` — releases are irreversible; confirm the inferred bump matches intent

---

## 7. Verification

**Done means** `npm run checks` exits 0 and the app boots without console errors.

```bash
npm run checks        # === scripts/checks.sh ===
```

Covers tsc, `biome check .` (the §3 TypeScript rules, mechanically — see `biome.jsonc`),
`cargo fmt --check`, clippy `--all-targets -D warnings` on the toolchain
pinned in `rust-toolchain.toml` (so local clippy == CI clippy), `cargo doc` for
intra-doc links, `scripts/check-doc-refs.sh`, the demeteo + core + runner test suites,
the gate-feedback repro, and commitlint on `origin/master..HEAD`.
Fails fast. "`cargo test` passed" is **not** "CI is green" — run the whole script, not
a subset. The `pre-push` hook runs it automatically (`git push --no-verify` for a
deliberate WIP).

### Comments are gated too

§3 says doc comments are the part worth keeping, which only holds while they are
still true. Nothing in a compiler reads a `//`, so two mechanical gates do:

- **`cargo doc`** resolves every `` [`Foo`] `` intra-doc link, denied via
  `[workspace.lints.rustdoc]`. A renamed item whose prose was not revisited fails
  the build. Clippy does **not** evaluate rustdoc lints — this gate is only
  reachable through `cargo doc`.
- **`scripts/check-doc-refs.sh`** resolves the file paths comments cite, and
  rejects a paragraph copied into more than one file.

The rot they exist to catch has one dominant cause: a module passes ~400 LOC,
`foo.rs` becomes `foo/`, and every comment naming `foo.rs` now points at nothing.
Because §3 makes that split routine, the pointers break routinely. The path gate
reports a ref only when something of that name exists elsewhere — the "it moved"
signature — so runtime artifact names in comments stay quiet without an allowlist.

When a rule applies repo-wide, **cite it — do not paste it**. Five copies of one
paragraph is five things to update and four that won't be. Shared rationale goes
on the module root (`domain/mod.rs`, `steps/mod.rs`) or in this file, and
submodules link to it.

**Working inside a Demeteo run, use `npm run checks:code` instead** — the same gates
without commitlint. Commitlint judges `origin/master..HEAD`, and inside a run that range
holds only orchestrator plumbing (one commit per ticket, plus subtask merges) that the
finalize step squashes away, having validated the surviving message against the real
`commit-msg` hook. A ticket agent cannot fix a message it never wrote, so the verdict
feeds a rework cycle that closes nothing. That is what `ProjectSettings.default_test_command`
should point at; `pre-push` and CI keep running the full `checks`.

**A new test does not count until you have watched it fail.** Break the code it
covers, confirm that test — and ideally only that test — goes red, then revert.
A suite that cannot fail is not coverage, and the shape that hides this is a
test double that answers every call successfully: the e2e `FakeExec` returns
`Ok("")` for every command, so anything *reading* git's output was being
asserted against a default rather than an answer. Prefer a double that errors
on anything it was not explicitly told to say.

### The parity gates are not in `npm run checks`

`pr-checks.yml` runs **three** jobs: `scripts/checks.sh`, plus two conformance suites
that `checks.sh` does not invoke. Both need Docker, and both are the only thing standing
between you and a local/remote divergence:

```bash
crates/demeteo-core/tests/conformance/run-ssh-conformance.sh       # C2.2 — same exec_contract, local vs loopback sshd
crates/demeteo-core/tests/conformance/run-topology-conformance.sh  # topology equivalence, local + SSH
```

Run them when you touch an `ExecutionPort` impl, the step executor, or anything a
transport observes. **The e2e suite will not catch this** — it drives a per-test
`FakeExec` that passes while masking exactly the drift these suites exist to find.

### Windows is invisible to every gate above

`checks.sh` compiles for the host only, so nothing it runs ever parses a
`#[cfg(windows)]` body past name resolution. That is how the first native-Windows
commit reached this tree carrying six Windows compile errors while local checks
were green. When you touch anything behind a `cfg(windows)`:

```bash
scripts/check-windows.sh        # cargo check, x86_64-pc-windows-gnu, via mingw-w64
scripts/check-windows.sh --run  # …and execute it under wine, where one exists
```

It skips with exit 0 when mingw-w64 or the rustup target is absent, so a green run
on a machine without them means *nothing was checked* — read its output, not just
its status. It uses the `gnu` target because MSVC's C dependencies do not
cross-compile from Linux; that shares the whole `cfg(windows)` surface but not MSVC
linkage, so it proves the source is coherent, not that the shipped artifact links.
`windows-latest` in `pr-checks.yml` stays the authority.

`--run` links that same target's test binary and executes it under wine, which
turns a compile check into a behaviour check for a large part of the surface —
but not for the artifact-scope fence, nor for anything reached through a login
shell, and it is local-only by construction. Read
[docs/WINDOWS_PARITY.md](docs/WINDOWS_PARITY.md) before trusting a green run;
what it cannot see is enumerated there, and a green there is never a substitute
for `windows-latest`.

The deeper rule this enforces: **a decision behind a `cfg` is a decision no local
test can reach.** Keep candidate ordering, path derivation, and probe interpretation
in `cfg`-free functions covered on Linux, and leave only the syscall behind the
`cfg` — see `shared/win/` for the shape. The inverse trap is easy to miss:
`cfg`-free logic proves nothing if its *fixtures* only parse on one platform. A
Windows-shaped path spelled with backslashes is a single filename everywhere
else, so a test built from one asserts nothing off Windows — and reads as a pass
wherever it expects `None`.

Fix any failure before handing back.

When your change has UI or runtime surface, smoke-test with `npm run dev:tauri` — **not**
`npm run tauri dev`. Only `dev:tauri` passes `--config src-tauri/tauri.dev.conf.json`,
which sets a separate app identifier (`com.stvcloud.demeteo.dev`) and keeps the dev
database isolated from the installed app. Use `npm run dev:tauri:sw` if the GPU path
misbehaves.

---

## 8. Documentation Index

Read the relevant doc before modifying that area.

| Area | Document |
|------|----------|
| Ports, adapters, hexagon, directory layout | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Domain model, ubiquitous language | [docs/DDD_MODEL.md](docs/DDD_MODEL.md) |
| 43 locked decisions (+ superseded) | [docs/DECISIONS.md](docs/DECISIONS.md) |
| Open & deferred questions | [docs/OPEN_QUESTIONS.md](docs/OPEN_QUESTIONS.md) |
| Agent CLI integration spec | [AGENT_INTEGRATION.md](AGENT_INTEGRATION.md) |
| Workflow DAG model, registry, canvas | [docs/PRD_DAG_WORKFLOWS.md](docs/PRD_DAG_WORKFLOWS.md) · remaining work in [docs/TASKS_DAG_WORKFLOWS.md](docs/TASKS_DAG_WORKFLOWS.md) |
| Local/remote execution parity | [docs/EXECUTION_PARITY.md](docs/EXECUTION_PARITY.md) |
| Windows parity plan & shell decision | [docs/WINDOWS_PARITY.md](docs/WINDOWS_PARITY.md) |
| Reliability invariants & open backlog | [docs/RELIABILITY_PLAN.md](docs/RELIABILITY_PLAN.md) |
| Harness truthfulness & baseline preflight | [docs/HARNESS_BASELINE.md](docs/HARNESS_BASELINE.md) |
| Remote execution design | [docs/REMOTE_EXECUTION.md](docs/REMOTE_EXECUTION.md) |
| Multi-client runner (designed, not built) | [docs/MULTI_CLIENT_RUNNER.md](docs/MULTI_CLIENT_RUNNER.md) |
| Remote-runner dev workflow & triage | [docs/RUNNER_DEV.md](docs/RUNNER_DEV.md) |
| Terminal agent activity | [docs/TERMINAL_ACTIVITY.md](docs/TERMINAL_ACTIVITY.md) |
| User stories & agent tasks | [docs/USER_STORIES.md](docs/USER_STORIES.md) |
| UX spec & journeys | [docs/UX_JOURNEYS.md](docs/UX_JOURNEYS.md) · as-built audit in [docs/ux-audit/](docs/ux-audit/README.md) |
| Product roadmap & agent-ready stories | [docs/roadmap/](docs/roadmap/README.md) |
| Known platform issues | [docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md) |
| Contributing, PR flow, full commit spec | [CONTRIBUTING.md](CONTRIBUTING.md) |
