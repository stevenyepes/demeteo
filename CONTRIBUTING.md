# Contributing to Demeteo

Thank you for your interest in contributing. This document covers the practical steps for submitting a bug report, feature request, or pull request.

## Before you start

Read [`AGENTS.md`](AGENTS.md) (the project constitution) and [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) before writing any code. Every architectural decision has a reason; working against the grain creates merge conflicts with existing and in-progress work.

## Reporting bugs

Open a GitHub issue with:
- Demeteo version (visible in Preferences → About)
- OS and version
- Steps to reproduce
- What you expected vs. what happened
- Relevant logs — `demeteo.log.<date>` (rotated daily) in `~/.local/share/com.stvcloud.demeteo/`
  (Linux, or `$XDG_DATA_HOME/com.stvcloud.demeteo/`), `%LOCALAPPDATA%\com.stvcloud.demeteo\`
  (Windows), or `~/Library/Application Support/com.stvcloud.demeteo.dev/` (macOS)

## Feature requests

Open a GitHub issue describing the use case, not just the feature. If a feature requires a new `npm` or `cargo` dependency, say so up front — dependency additions require explicit approval (see [§6 of AGENTS.md](AGENTS.md)).

## Pull requests

### Setup

Prerequisites:

- **Node.js 24** — the version every CI workflow uses.
- **Rust via rustup** — `rust-toolchain.toml` pins the toolchain (currently 1.97.0, with
  `clippy` and `rustfmt`); rustup installs it on first use, so don't override it.
- **Tauri v2 system dependencies** for your OS — see the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). On Debian/Ubuntu that is
  the list `pr-checks.yml` installs (`libwebkit2gtk-4.1-dev`, `libappindicator3-dev`,
  `librsvg2-dev`, `patchelf`, `libssl-dev`, `libdbus-1-dev`, `pkg-config`).

```bash
git clone https://github.com/stevenyepes/demeteo
cd demeteo
npm install          # also wires the git hooks (core.hooksPath .githooks)
npm run dev:tauri    # run the app against an isolated dev database
```

Use `npm run dev:tauri`, not `npm run tauri dev`: only the former passes
`src-tauri/tauri.dev.conf.json`, which gives the dev build its own app identifier
(`com.stvcloud.demeteo.dev`) so it never touches the installed app's database. If the GPU
path misbehaves, use `npm run dev:tauri:sw`.

### Branching

Branch from `master`:

```bash
git checkout -b your-name/short-description
```

### Code conventions

Follow the conventions in [§3 of AGENTS.md](AGENTS.md). Key points:

**TypeScript / React**
- One component per file; extract when a file passes ~400 LOC
- No `any` — use `unknown` + a type guard if the shape is uncertain
- All Tauri commands called through typed wrappers in `src/lib/` — never call `invoke()` raw in a component

**Rust**
- No `.unwrap()` or `.expect()` in production paths — use `?` or match
- Run `cargo fmt --all` before committing; clippy runs as `--workspace --all-targets -- -D warnings`
  on the pinned toolchain — `npm run checks` runs it exactly as CI does
- DB access goes through the repositories in `crates/demeteo-core/src/adapters/database/` — no raw `rusqlite` calls in commands

### Commit messages

Every commit must follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/).
This is enforced twice: the `commit-msg` git hook (wired by `npm install` via
`core.hooksPath .githooks`) rejects a bad message locally, and the
[`Lint Commits`](.github/workflows/lint-commits.yml) workflow runs commitlint in CI.

```
<type>(<optional-scope>): <subject>

<body>

<footer>
```

- **Subject** ≤ 72 chars, lower-case first letter, imperative mood, no trailing period.
- **Type** is mandatory and lower-case. **Scope** is optional and lower-case (`orchestrator`, `settings`, `ci`).
- **Body** explains *why*, wrapped at 100 cols, separated from the subject by a blank line.
- **Footer** carries `BREAKING CHANGE: <note>` for any non-backwards-compatible change.

Release automation reads these to infer the next version, so the type matters:

| Type | Bump | When to use |
|------|------|-------------|
| `feat` | minor | A new user-facing feature |
| `fix` | patch | A bug fix |
| `perf` | patch | Performance improvement, no behaviour change |
| `revert` | patch | Reverts a previous commit |
| `refactor` | none | Internal change, no behaviour shift |
| `docs` | none | Documentation only |
| `style` | none | Formatting / whitespace |
| `test` | none | Adding or fixing tests |
| `build` | none | Build system, dependencies, external tooling |
| `ci` | none | CI / GitHub Actions configuration |
| `chore` | none | Tooling, scripts, maintenance, release bumps |

A commit signals a **major** bump when the type carries a `!`
(`feat(api)!: drop legacy v0 endpoints`) or the body/footer has a `BREAKING CHANGE:`
line. Across a range the highest bump wins; an unrecognized type defaults to patch.

```
✅ feat(orchestrator): add parallel step fan-out
✅ fix(settings): guard against null provider url
✅ feat(api)!: drop legacy v0 endpoints
❌ Fix bug                        — wrong case, vague
❌ Updated stuff                  — no type
❌ feat: Added a thing.           — past tense + trailing period
❌ feat(remote): P0 multi-client runner
     — subject starts with a capitalized token; commitlint's `subject-case`
       rejects it. A leading acronym, ticket id, or TypeName trips this too.
       Start with a lower-case word: `feat(remote): multi-client runner P0`.
```

Check a message before committing:

```bash
echo "<your commit message>" | npx commitlint
```

### Verification checklist

Before opening a PR, run the full gate:

```bash
npm run checks
```

This is `scripts/checks.sh`, the same script CI runs: [`pr-checks.yml`](.github/workflows/pr-checks.yml)
invokes it as two jobs (`frontend`, `rust`), and commitlint runs in
[`Lint Commits`](.github/workflows/lint-commits.yml). It covers `tsc --noEmit`, `biome check .`,
the class-name gate (`scripts/check-classes.mjs`), the Vitest frontend suite,
`cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings` on the pinned
toolchain, `cargo doc` intra-doc links, `scripts/check-doc-refs.sh`, the demeteo + core +
runner test suites, and commitlint over `origin/master..HEAD`. Running a subset —
`cargo test` alone, say — will not tell you whether CI is green.

Nothing runs it for you: the `pre-push` hook is deliberately inert, so run it before
pushing. `scripts/checks.sh frontend` and `scripts/checks.sh rust` are the two halves CI
runs as separate jobs, useful while iterating; the no-argument form is the gate.

Two things `npm run checks` cannot see, both covered in [§7 of AGENTS.md](AGENTS.md): the
Docker-based `ExecutionPort` conformance suites (run them when you touch an execution
transport or the step executor), and `#[cfg(windows)]` code (`scripts/check-windows.sh`).
CI's `cross-os` job type-checks and runs the core tests on macOS and Windows.

If your change has UI or runtime surface, also confirm the app boots clean:

```bash
npm run dev:tauri   # no console errors
```

### Opening the PR

- **Write the title as a conventional commit subject** — a squash merge lands it verbatim as the
  commit on `master`, where the release bump is inferred from it, and the `Lint Commits` workflow
  checks it on every PR. GitHub appends ` (#N)`, which counts against the 72-character subject
  limit, so keep it under about 64 characters and use the description for context
- Reference any related issue with `Closes #N`
- If your change touches a Gate-policy area (migrations, Tauri capabilities, agent spawn logic, worktree merge), say so explicitly in the PR description
- Every PR runs the [`PR Checks` workflow](.github/workflows/pr-checks.yml) — the same `scripts/checks.sh` as `npm run checks` above (as `frontend` and `rust` jobs), plus the `cross-os` and `conformance` jobs, so a `frontend` or `rust` failure you see inline on the PR reproduces locally. It runs on `master` too, after every merge, and that run is what warms the Rust caches every PR reads.
- Commit subjects and the PR title are linted by [`Lint Commits`](.github/workflows/lint-commits.yml); the title is what a squash merge becomes, so keep it under 72 characters and conventional.

## What we won't merge

- Changes that break the hexagonal architecture (business logic in `commands/`, adapters called directly from React)
- New dependencies added without prior discussion
- `.unwrap()` / `.expect()` in Rust command handlers or domain logic
- Credentials, tokens, or secrets written to SQLite or any file
- Hard-coded `localhost`, port numbers, or paths

## License

By contributing you agree that your changes are licensed under the [MIT License](LICENSE).
