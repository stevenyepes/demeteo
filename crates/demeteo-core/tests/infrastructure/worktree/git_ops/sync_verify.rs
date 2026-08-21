// Tests for `src/adapters/worktree/git_ops/sync_verify.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// The claim under test is not "a red build blocks the push" — that half is a
// one-line match. It is the three cases that must *not* block one, because
// each of them withholds a merge already committed on the user's branch on the
// strength of something that is not a verdict about it.

use super::*;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};

const WT: &str = "/repos/demeteo_wt_sync_feature";

fn opts() -> ShellOptions {
    ShellOptions {
        cwd: Some(WT.to_string()),
        ..ShellOptions::login_interactive()
    }
}

fn gate<'a>(prepare: Option<&'a str>, harness: Option<&'a str>) -> MergeGate<'a> {
    MergeGate { prepare, harness }
}

#[tokio::test]
async fn a_red_harness_withholds_the_push_and_says_which_command_went_red() {
    let exec = ScriptedExec::new(&[(
        "npm run checks:code",
        Err("Command failed (exit code: Some(101)): error[E0063]: missing fields"),
    )]);

    let refusal = merge_gate_refusal(
        &exec,
        "local",
        gate(None, Some("npm run checks:code")),
        opts(),
    )
    .await
    .expect("a harness that answered red withholds the push");

    assert!(
        refusal.contains("npm run checks:code"),
        "the refusal has to name the command that failed, or the user cannot \
         reproduce it: {refusal}"
    );
    assert!(
        refusal.contains("E0063"),
        "git's and the harness's own words are the half the session row cannot \
         reconstruct: {refusal}"
    );
}

#[tokio::test]
async fn a_project_that_names_no_harness_runs_nothing_and_withholds_nothing() {
    // Scripted with *no* answers: anything this double is asked answers `Err`,
    // so "it ran a command anyway" fails here rather than passing quietly.
    let exec = ScriptedExec::new(&[]);

    assert!(
        merge_gate_refusal(&exec, "local", gate(Some("npm ci"), None), opts())
            .await
            .is_none(),
        "an absent gate is not a passed gate, but it withholds nothing"
    );
    assert!(
        exec.commands().is_empty(),
        "no harness means no prepare either: {:?}",
        exec.commands()
    );
}

/// A build that never ran is not a red build. Both of these arrive as `Err`
/// from the same call the red one does, and only the prefix separates them.
#[tokio::test]
async fn a_harness_nobody_could_run_does_not_withhold_a_committed_merge() {
    for err in [
        format!("{TRANSPORT_ERROR_PREFIX} ssh channel closed"),
        format!("{TIMEOUT_ERROR_PREFIX} exceeded 1800s"),
    ] {
        let exec = ScriptedExec::new(&[("npm run checks:code", Err(err.as_str()))]);

        assert_eq!(
            merge_gate_refusal(
                &exec,
                "local",
                gate(None, Some("npm run checks:code")),
                opts()
            )
            .await,
            None,
            "a harness abandoned with {err:?} says nothing about the merge, so \
             withholding it would strand a real commit on the branch"
        );
    }
}

/// The false red this gate would otherwise manufacture on every JS project: a
/// sync worktree whose install failed reports a missing-module build error that
/// is about the worktree, not about the merge.
#[tokio::test]
async fn a_failed_prepare_skips_the_gate_rather_than_blaming_the_merge() {
    let exec = ScriptedExec::new(&[
        ("npm ci", Err("npm ERR! network ETIMEDOUT")),
        (
            "npm run checks:code",
            Err("Cannot find module 'react' — this must not be reached"),
        ),
    ]);

    assert_eq!(
        merge_gate_refusal(
            &exec,
            "local",
            gate(Some("npm ci"), Some("npm run checks:code")),
            opts()
        )
        .await,
        None,
        "a worktree that could not be prepared cannot answer the question, and \
         'your merge is broken' is the wrong answer to that"
    );
    assert_eq!(
        exec.commands(),
        vec!["npm ci".to_string()],
        "the harness must not run after a failed prepare: its verdict would be \
         about the missing install"
    );
}

#[tokio::test]
async fn a_green_harness_runs_after_its_prepare_and_lets_the_push_through() {
    let exec = ScriptedExec::new(&[
        ("npm ci", Ok("added 900 packages")),
        ("npm run checks:code", Ok("all checks passed")),
    ]);

    assert_eq!(
        merge_gate_refusal(
            &exec,
            "local",
            gate(Some("npm ci"), Some("npm run checks:code")),
            opts()
        )
        .await,
        None
    );
    assert_eq!(
        exec.commands(),
        vec!["npm ci".to_string(), "npm run checks:code".to_string()],
        "prepare first, and both of them in the merge worktree"
    );
}

/// The gate runs where the merge is, under the shell a user-authored command
/// needs. A gate that ran in the clone would test the branch the sync worktree
/// was cut *from*, and one under a bare `sh -c` would report "command not
/// found" for every `mise`/`nvm`-shimmed toolchain.
#[tokio::test]
async fn the_gate_runs_in_the_merge_worktree_under_the_harness_shell() {
    let exec = ScriptedExec::new(&[("cargo test", Ok(""))]);

    merge_gate_refusal(&exec, "local", gate(None, Some("cargo test")), opts()).await;

    let seen = exec
        .options()
        .into_iter()
        .next()
        .expect("the harness was invoked");
    assert_eq!(seen.cwd.as_deref(), Some(WT));
    assert!(seen.login_shell && seen.interactive);
}
