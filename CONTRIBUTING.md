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
- Relevant logs from `~/.local/share/demeteo/` (Linux) or the platform equivalent

## Feature requests

Open a GitHub issue describing the use case, not just the feature. If a feature requires a new `npm` or `cargo` dependency, say so up front — dependency additions require explicit approval (see [§7 of AGENTS.md](AGENTS.md)).

## Pull requests

### Setup

```bash
git clone https://github.com/stevenyepes/demeteo
cd demeteo
npm install
```

### Branching

Branch from `master`:

```bash
git checkout -b your-name/short-description
```

### Code conventions

Follow the conventions in [§4 of AGENTS.md](AGENTS.md). Key points:

**TypeScript / React**
- Named exports only — no default exports
- No `any` — use `unknown` + a type guard if the shape is uncertain
- All Tauri commands called through typed wrappers in `src/lib/` — never call `invoke()` raw in a component

**Rust**
- No `.unwrap()` or `.expect()` in production paths — use `?` or match
- Run `cargo fmt` and `cargo clippy -- -D warnings` before committing
- DB access goes through `src-tauri/src/db.rs` — no raw `rusqlite` calls in commands

### Verification checklist

Before opening a PR, confirm all three pass:

```bash
# Frontend type-check
npx tsc --noEmit

# Rust format + lint
cd src-tauri && cargo fmt && cargo clippy -- -D warnings && cd ..

# App boots without console errors
npm run dev:tauri
```

### Opening the PR

- Keep the title short (under 70 characters) and use the description for context
- Reference any related issue with `Closes #N`
- If your change touches a Gate-policy area (migrations, Tauri capabilities, agent spawn logic, worktree merge), say so explicitly in the PR description

## What we won't merge

- Changes that break the hexagonal architecture (business logic in `commands/`, adapters called directly from React)
- New dependencies added without prior discussion
- `.unwrap()` / `.expect()` in Rust command handlers or domain logic
- Credentials, tokens, or secrets written to SQLite or any file
- Hard-coded `localhost`, port numbers, or paths

## License

By contributing you agree that your changes are licensed under the [MIT License](LICENSE).
