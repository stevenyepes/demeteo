//! Which binaries a project's configured commands will actually try to run.
//!
//! Pure over strings and one `WorktreeStrategy`: no port double, no async
//! runtime. Moved out of `tests/infrastructure/step_executor/preflight_tests.rs`
//! with their assertions unchanged when the policy moved to `domain/`.

use super::*;

#[path = "../../support/preflight_strategy.rs"]
mod preflight_strategy;
use preflight_strategy::strategy;

#[test]
fn a_plain_command_yields_its_binary() {
    assert_eq!(probeable_binaries(&["cargo test"]), vec!["cargo"]);
}

#[test]
fn every_stage_of_a_chain_is_probed() {
    // The real shape from the dev DB. Each `&&` stage runs a different tool,
    // and any one of them missing breaks the whole harness — so probing only
    // the first would miss exactly the polyglot case that motivated this.
    assert_eq!(
        probeable_binaries(&[
            "npx vitest run && npm run build && cargo build --manifest-path src-tauri/Cargo.toml",
        ]),
        vec!["npx", "npm", "cargo"]
    );
}

#[test]
fn a_repeated_binary_is_probed_once() {
    assert_eq!(
        probeable_binaries(&["cargo fmt && cargo clippy && cargo test"]),
        vec!["cargo"]
    );
}

#[test]
fn leading_env_assignments_are_stepped_over() {
    // `RUST_LOG=debug cargo test` runs `cargo`. Probing `RUST_LOG=debug` would
    // never resolve and would block a perfectly good launch.
    assert_eq!(
        probeable_binaries(&["RUST_LOG=debug CI=1 cargo test"]),
        vec!["cargo"]
    );
}

#[test]
fn shell_builtins_are_not_probed() {
    // `cd src-tauri && cargo test` — the exact command this project's own
    // settings carried. `cd` is a builtin; whether `command -v cd` answers is
    // shell-dependent and irrelevant.
    assert_eq!(
        probeable_binaries(&["cd src-tauri && cargo test"]),
        vec!["cargo"]
    );
}

#[test]
fn what_detection_emits_for_a_polyglot_repo_probes_only_real_tools() {
    // `detect_worktree_strategy` used to emit one hand-rolled accumulator for a
    // multi-ecosystem repo — `set +e; rc=0; npm test; rc=$((rc||$?)); …` — and
    // this test pinned it. HB3 deleted that string in favour of named
    // harnesses, but the property it was covering survives: detection still
    // emits multi-command strings, and the preflight must find the real tools in
    // them without being fooled by the shell around them.
    //
    // The two shapes it now emits. Everything except `npm` and `cargo` is a
    // builtin or a parenthesis — the subshell exists so a `cd` cannot leak into
    // the next link of the chain, and `(cd` must not be probed as a binary.
    assert_eq!(
        probeable_binaries(&["npm ci && (cd src-tauri && cargo fetch)"]),
        vec!["npm", "cargo"]
    );
    assert_eq!(
        probeable_binaries(&["(cd src-tauri && cargo test)"]),
        vec!["cargo"]
    );
    // And the corrected watch-mode form, whose `--` separator must not be read
    // as the start of a new command.
    assert_eq!(probeable_binaries(&["npm test -- --run"]), vec!["npm"]);
}

#[test]
fn unresolvable_words_are_skipped_rather_than_guessed_at() {
    // A false positive blocks a legitimate launch; a false negative just
    // lands the user in today's behaviour. Anything needing a shell to
    // resolve is therefore dropped.
    assert!(probeable_binaries(&["$(which pytest) -q"]).is_empty());
    assert!(probeable_binaries(&["`echo cargo` test"]).is_empty());
    assert!(probeable_binaries(&["./scripts/*.sh"]).is_empty());
}

#[test]
fn an_empty_or_whitespace_command_probes_nothing() {
    assert!(probeable_binaries(&[""]).is_empty());
    assert!(probeable_binaries(&["   \n  "]).is_empty());
}

// ── HB4: the union of every configured command ──────────────────────────────

#[test]
fn the_union_dedupes_across_commands_and_keeps_declaration_order() {
    // The point of lifting the dedupe out of a single command: prepare, test
    // and a harness all reaching for `npm` is one `command -v`, not three.
    assert_eq!(
        probeable_binaries(&["npm ci", "npm test && cargo test", "npx playwright test"]),
        vec!["npm", "cargo", "npx"]
    );
}

#[test]
fn configured_commands_are_ordered_prepare_test_then_harnesses_by_name() {
    // `harnesses` is a `HashMap`. Without the sort the probe order — and the
    // order binaries appear in a blocking message — would differ run to run.
    let s = strategy(
        Some("npm ci"),
        Some("npm test"),
        &[("unit", "npm run unit"), ("integration", "npm run e2e")],
    );
    assert_eq!(
        configured_commands(&s),
        vec!["npm ci", "npm test", "npm run e2e", "npm run unit"]
    );
}
