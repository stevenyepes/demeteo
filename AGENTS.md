# Demeteo — Agent Constitution

> **You are working on a fleet-style multi-agent orchestrator** built with
> Tauri v2 (Rust) + React 19 (TypeScript). Read this file top-to-bottom
> before writing any code. Every section is mandatory unless marked *(optional)*.
>
> **Before writing a single line of code, you must complete the thinking
> protocol in Section 0.** Skipping it is not allowed.

---

## 0. Mandatory Thinking Protocol

> **Complete this before opening any file to edit.**

For every task, reason through the following in order — write your answers
as a short scratchpad response before producing any code:

1. **Locate the layer.** Which layer does this change live in?
   - `domain/` (pure logic, no I/O)
   - `ports/` (trait definitions)
   - `adapters/` (port implementations)
   - `commands/` (thin IPC handlers)
   - `src/lib/` (typed frontend wrappers)
   - `src/components/` (React UI)

2. **Map the ripple.** List every file that will need to change as a
   consequence — including types, IPC wrappers, and tests.

3. **Check the hexagon.** Confirm the change does not:
   - Put business logic in a `commands/` handler
   - Call an adapter directly from a React component
   - Cross a layer boundary that ports are meant to abstract

4. **Identify the Gate.** Does this touch a Gate-policy area
   (migrations, capabilities, agent spawn, worktree merge)?
   If yes, stop and ask the user before proceeding.

5. **State your plan.** One sentence per file: what changes and why.

Only after completing steps 1–5 may you write or modify code.

---

## Quick Reference

| What                    | Command                                      |
|-------------------------|----------------------------------------------|
| Start dev app           | `npm run tauri dev`                          |
| Frontend only           | `npm run dev`                                |
| Build frontend          | `npm run build`                              |
| Type-check              | `npx tsc --noEmit`                           |
| Rust check              | `cargo check` (inside `src-tauri/`)          |
| Rust fmt                | `cargo fmt` (inside `src-tauri/`)            |
| Rust clippy             | `cargo clippy -- -D warnings`               |

**Done means:** `tsc --noEmit` exits 0, `cargo clippy` exits 0, the app boots without console errors.

---

## 1. Project Identity

**Demeteo** is a premium desktop app that lets a developer describe a feature in plain language; the app decomposes it into a Workflow, delegates Steps to coding agents (opencode, claude-code, hermes, antigravity), manages Git worktrees per Step, and presents human-approval Gates before merging.

> **Current phase: V1 — Core fleet-style multi-agent orchestrator** (fully implemented).

**Core vocabulary** *(use these exact names in code and comments)*:

| Term               | Meaning                                                                 |
|--------------------|-------------------------------------------------------------------------|
| `Project`          | A local or remote Git repo tracked by Demeteo                          |
| `Feature`          | A user-described piece of work decomposed by a Workflow                |
| `Workflow`         | Reusable, versioned DAG of Steps                                        |
| `Step`             | One node in the DAG: `agent`, `parallel`, or `gate`                    |
| `Subtask`          | Work assigned to one agent in one worktree                             |
| `Gate`             | Human-approval checkpoint before the orchestrator continues            |
| `ProviderInstance` | A configured AI provider (model + key + endpoint)                      |

---

## 2. Tech Stack

### Key constraints
- `external_directory: "deny"` — agents are scoped to their worktree; never allow FS access outside it
- Agent integration is **one-shot CLI + JSON only** — no ACP, no JSON-RPC, no tool-call bridge
- Secrets live in the OS keyring only — never write credentials to SQLite or disk files

---

## 3. Architecture in 30 Seconds

```
React Webview ──IPC──► Tauri Commands ──► FeatureOrchestrator
                                              │
                          ┌───────────────────┤
                          ▼                   ▼
                    AgentRuntime        WorktreeManager
                    (CliRuntime)        (MergeExecutor)
                          │                   │
                  opencode / hermes     Git worktrees
                  claude-code / ag      SSH/SFTP repos
```

Frontend components → Tauri IPC → Rust core → SQLite + OS + Agents

For the full hexagon, port catalogue, and directory layout → [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

---

## 4. Code Conventions

### TypeScript / React
- Named exports only — no default exports
- File names: `PascalCase.tsx` for components, `camelCase.ts` for utilities
- One component per file; keep files under ~400 LOC — extract when larger
- All Tauri commands called through typed wrappers in `src/lib/` — never call `invoke()` raw in components
- `async/await` everywhere — no raw `.then()` chains
- No `any` types — use `unknown` + a type guard if the shape is uncertain
- Prefer `interface` over `type` for object shapes; use `type` for unions/aliases

### Rust
- Return `Result<T, String>` from `#[tauri::command]` functions — map errors with `.map_err(|e| e.to_string())`
- Use `thiserror` for domain error enums in `src-tauri/src/domain/`
- All DB access goes through `src-tauri/src/db.rs` — no raw `rusqlite` calls in commands
- Never use `.unwrap()` or `.expect()` in production paths — use `?` or match
- Format: `cargo fmt` before every commit; lint: `cargo clippy -- -D warnings` must be clean

### Naming
- Rust structs/enums: `PascalCase`; functions/variables: `snake_case`
- React components: `PascalCase`; hooks: `useCamelCase`; event handlers: `handleCamelCase`
- Tauri command names: `snake_case` (e.g., `create_project`, `start_feature`)

---

## 5. Visual Design Rules

> Every UI change **must** follow these rules without exception.

| Token        | Value                      | Semantic use                          |
|--------------|----------------------------|---------------------------------------|
| Background   | `#08090c` / `#0d0f14`      | App shell, page backgrounds           |
| Card surface | `rgba(18,22,30,0.75)`      | Glassmorphism panels                  |
| Border glow  | `rgba(255,255,255,0.05)`   | Card borders                          |
| Violet       | `#8b5cf6`                  | Active connections, primary actions   |
| Cyan         | `#06b6d4`                  | Terminal streams, interactive states  |
| Emerald      | `#10b981`                  | Running agents, healthy statuses      |
| Ruby         | `#ef4444`                  | Errors, stopped tasks, failures       |

- **Cards**: `backdrop-filter: blur(12px)` + `rgba(18,22,30,0.75)` background
- **Typography**: headings → `Outfit`; UI text → `Inter`; terminal/code → `Fira Code` / `JetBrains Mono`
- **Motion**: pulsing glows for status dots; smooth transitions on view switches — no jarring snaps
- **Never**: plain system colors, `style=` props for design tokens, static grey cards with no depth

---

## 6. File Layout (active code)

```
demeteo/
├── src/                        # React frontend
│   ├── components/             # One file = one component
│   ├── hooks/                  # Custom React hooks
│   ├── lib/                    # Tauri IPC wrappers, utilities
│   ├── types.ts                # Shared TypeScript types
│   └── App.tsx                 # Root router / layout
├── src-tauri/
│   ├── src/
│   │   ├── commands/           # #[tauri::command] handlers (thin)
│   │   ├── application/        # Use cases / application services (NEW)
│   │   ├── domain/             # Domain structs, enums, errors
│   │   ├── ports/              # Trait definitions (hexagon ports)
│   │   ├── adapters/           # Port implementations
│   │   ├── infrastructure/     # Infrastructure code (e.g., SSH resolver, agent event stream)
│   │   ├── db.rs               # DB connection + query helpers
│   │   ├── state.rs            # AppState (Mutex-wrapped shared state)
│   │   └── lib.rs              # Plugin registration, command registration
│   └── migrations/             # SQL migration files (refinery)
└── docs/                       # Architecture & design docs (read-only for agents)
```

> **Do not** create files outside this structure without first updating this layout map.

---

## 7. Negative Constraints

Things an agent must **never** do without explicit user approval:

- ❌ Add a new `npm` or `cargo` dependency
- ❌ Delete or rename existing migration files in `src-tauri/migrations/`
- ❌ Write credentials, tokens, or secrets to SQLite or any file
- ❌ Call `invoke()` directly in a React component — use a typed wrapper in `src/lib/`
- ❌ Use `unwrap()` / `expect()` in Rust command handlers or domain logic
- ❌ Hard-code `localhost`, port numbers, or paths — read from config/state
- ❌ Break the hexagon: commands must not contain business logic; adapters must not be called from the frontend directly
- ❌ Skip `cargo fmt` + `cargo clippy` before finalizing Rust changes
- ❌ Bypass the `PermissionPolicyPort` when spawning agent processes
- ❌ Create a git commit whose message does not conform to the Conventional Commits format
  in **Section 8** — no free-form messages, no past tense, no trailing period, no missing type

---

## 8. Commit Convention (mandatory)

All commits on `master` **MUST** follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/).
This rule is enforced by the [`Lint Commits`](.github/workflows/lint-commits.yml)
GitHub Actions workflow using commitlint, and by the release automation that
reads commit messages to infer the next version bump.

### Format

```
<type>(<optional-scope>): <subject>

<body>

<footer>
```

- **Subject** ≤ 72 chars, lower-case first letter, imperative mood, no trailing period.
- **Type** is mandatory and lower-case.
- **Scope** is optional and lower-case (e.g. `orchestrator`, `settings`, `ci`).
- **Body** explains *why*, wraps at 100 cols, separated from subject by a blank line.
- **Footer** carries `BREAKING CHANGE: <note>` for any non-backwards-compatible change.

### Allowed types and their semver effect

| Type       | Bump   | When to use                                                |
|------------|--------|------------------------------------------------------------|
| `feat`     | minor  | A new user-facing feature                                  |
| `fix`      | patch  | A bug fix                                                  |
| `perf`     | patch  | Performance improvement with no behaviour change           |
| `revert`   | patch  | Reverts a previous commit                                  |
| `refactor` | none   | Internal change, no behaviour shift                        |
| `docs`     | none   | Documentation only                                         |
| `style`    | none   | Formatting / whitespace; no logic change                   |
| `test`     | none   | Adding or fixing tests                                     |
| `build`    | none   | Build system, dependencies, or external tooling            |
| `ci`       | none   | CI / GitHub Actions configuration                          |
| `chore`    | none   | Tooling, scripts, repository maintenance, release bumps    |

A commit also signals a **major** bump when:

- the type is suffixed with `!` → `feat(api)!: drop legacy v0 endpoints`, **or**
- the body / footer contains a `BREAKING CHANGE:` line.

When several types appear in the range, the **highest** bump wins
(major > minor > patch). If nothing matches a known type, the release
defaults to **patch** (something clearly changed).

### Examples

- ✅ `feat(orchestrator): add parallel step fan-out`
- ✅ `fix(settings): guard against null provider url`
- ✅ `perf(terminal): debounce xterm output writes`
- ✅ `docs(readme): clarify macOS quarantine workaround`
- ✅ `feat(api)!: drop legacy v0 endpoints`
- ✅ `feat(settings): redesign preferences screen`
- ❌ `Fix bug` — wrong case, no scope, vague
- ❌ `Updated stuff` — no type
- ❌ `feat: Added a thing.` — past tense + trailing period

### Release automation that depends on this

| Workflow                              | What it reads from commits              |
|---------------------------------------|------------------------------------------|
| `Lint Commits`                        | Subject format (commitlint)              |
| `Build & Release` (master pushes)     | Triggers an rc counter bump              |
| `Promote Release` (manual dispatch)   | Infers the next semver from commit types  |

The `Promote Release` workflow **suggests** a version (e.g.
`0.1.0 → 0.2.0 (minor) · feat=3 · fix=7 · breaking=1`) and
auto-applies it; an `override_bump_type` input is available as a safety valve.

---

## 9. Gate Policy (Human Approval)

Any task that touches the items below requires a **Gate** (pause and ask the user) before executing:

- Schema migrations that drop or rename columns
- Changes to `src-tauri/capabilities/` (Tauri permission surfaces)
- Changes to agent spawn logic or `OPENCODE_PERMISSION` env construction
- Merging worktrees back to a feature branch when conflicts are detected
- **Re-running the `Promote Release` workflow** — releases are not reversible; confirm the inferred bump matches intent (use the override input only when justified)

---

## 10. Documentation Index

> Read the relevant doc before modifying the corresponding area.

| Area                        | Document                                                                              |
|-----------------------------|---------------------------------------------------------------------------------------|
| Domain model (ubiquitous language, aggregates) | [docs/DDD_MODEL.md](docs/DDD_MODEL.md)                              |
| Ports, adapters, directory layout | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)                                       |
| 36 locked decisions         | [docs/DECISIONS.md](docs/DECISIONS.md)                                                 |
| Open & deferred questions   | [docs/OPEN_QUESTIONS.md](docs/OPEN_QUESTIONS.md)                                       |
| Agent CLI integration spec  | [AGENT_INTEGRATION.md](AGENT_INTEGRATION.md)                                           |
| Reliability & DAG pipeline  | [docs/RELIABILITY_PLAN.md](docs/RELIABILITY_PLAN.md)                                   |
| User stories & agent tasks  | [docs/USER_STORIES.md](docs/USER_STORIES.md)                                           |
| UX spec & journeys          | [docs/UX_JOURNEYS.md](docs/UX_JOURNEYS.md)                                             |

---

## 11. Verification Checklist

Run this before marking any task done:

```bash
# Frontend
npx tsc --noEmit

# Rust
cd src-tauri && cargo fmt && cargo clippy -- -D warnings && cd ..

# App boots
npm run tauri dev   # open the app, confirm no console errors
```

If any step fails, fix it before handing back to the user.

### Commit message (run before every `git commit`)

Every commit **must** pass commitlint locally:

```bash
echo "<your commit message>" | npx commitlint
```

Use this exact shell form to commit — never free-form text:

```bash
git commit -m "<type>(<scope>): <subject>"
# e.g.
git commit -m "fix(orchestrator): handle null provider url on startup"
```

If `npx commitlint` exits non-zero, rewrite the message before committing.
Valid types: `feat fix perf revert refactor docs style test build ci chore`

---


