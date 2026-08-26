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

    assert_eq!(
        refusal,
        "The merge is committed on this branch and the project's checks failed in it, so it \
         was not pushed.\n\n$ npm run checks:code\nCommand failed (exit code: Some(101)): \
         error[E0063]: missing fields",
        "the command that failed and what it said are what the user reproduces from, and \
         nothing else holds them once the sync worktree is gone"
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
///
/// A verdict of its own rather than the one a green harness answers, because
/// the conflicted half decides more from it than "push" — it decides what to
/// promise the resolver about a command that will not run.
#[tokio::test]
async fn a_failed_prepare_is_unprepared_not_a_pass() {
    let script = [
        ("npm ci", Err("npm ERR! network ETIMEDOUT")),
        (
            "npm run checks:code",
            Err("Cannot find module 'react' — this must not be reached"),
        ),
    ];
    let exec = ScriptedExec::new(&script);

    assert_eq!(
        run_merge_gate(
            &exec,
            "local",
            gate(Some("npm ci"), Some("npm run checks:code")),
            opts(),
            None,
        )
        .await,
        GateVerdict::Unprepared {
            error: "npm ERR! network ETIMEDOUT".to_string()
        },
    );
    assert_eq!(
        exec.commands(),
        vec!["npm ci".to_string()],
        "the harness must not run after a failed prepare: its verdict would be \
         about the missing install"
    );

    let exec = ScriptedExec::new(&script);
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
}

/// Three things reached one word, and a caller that *creates* a merge commit
/// on the strength of it needs them apart: only one of them is a tree anybody
/// looked at.
#[tokio::test]
async fn the_three_ways_nothing_withholds_a_push_are_three_verdicts() {
    let cases = [
        (
            "no harness",
            gate(Some("npm ci"), None),
            [].as_slice(),
            GateVerdict::NotGated,
        ),
        (
            "green harness",
            gate(None, Some("cargo test")),
            [("cargo test", Ok("ok"))].as_slice(),
            GateVerdict::Passed,
        ),
        (
            "failed prepare",
            gate(Some("npm ci"), Some("cargo test")),
            [("npm ci", Err("ETIMEDOUT"))].as_slice(),
            GateVerdict::Unprepared {
                error: "ETIMEDOUT".to_string(),
            },
        ),
    ];

    for (what, g, script, expected) in cases {
        let exec = ScriptedExec::new(script);
        assert_eq!(
            run_merge_gate(&exec, "local", g, opts(), None).await,
            expected,
            "{what}"
        );

        let exec = ScriptedExec::new(script);
        assert_eq!(
            merge_gate_refusal(&exec, "local", g, opts()).await,
            None,
            "{what} withholds nothing from a merge already on the branch"
        );
    }
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

/// Stop has to reach a build already running, and the only thing that stops one
/// is dropping the future the adapter kills the process group on.
///
/// The double is scripted with nothing, so a gate that ran the harness anyway
/// comes back `Failed` with "unscripted command" rather than reading as a pass.
#[tokio::test]
async fn a_stop_mid_gate_is_not_a_red_build() {
    let exec = ScriptedExec::new(&[]);
    let (_tx, rx) = tokio::sync::watch::channel(true);

    assert_eq!(
        run_merge_gate(
            &exec,
            "local",
            gate(Some("npm ci"), Some("npm run checks:code")),
            opts(),
            Some(rx),
        )
        .await,
        GateVerdict::Stopped,
    );
    assert!(
        exec.commands().is_empty(),
        "a stopped gate runs nothing: {:?}",
        exec.commands()
    );

    let (_tx, rx) = tokio::sync::watch::channel(true);
    assert_eq!(
        run_gate_prepare(
            &exec,
            "local",
            gate(Some("npm ci"), Some("npm run checks:code")),
            opts(),
            Some(rx),
        )
        .await,
        GatePrepare::Stopped,
        "the half the conflicted caller runs on its own answers it too"
    );
}

/// The cancel race is wrapped around *both* commands, and prepare is the one a
/// rewrite can drop silently: nothing downstream of it reports that it ran.
#[tokio::test]
async fn the_prepare_command_still_runs_under_the_cancel_race() {
    let exec = ScriptedExec::new(&[
        ("npm ci", Ok("added 900 packages")),
        ("npm run checks:code", Ok("all checks passed")),
    ]);

    assert_eq!(
        run_merge_gate(
            &exec,
            "local",
            gate(Some("npm ci"), Some("npm run checks:code")),
            opts(),
            None,
        )
        .await,
        GateVerdict::Passed,
    );
    assert_eq!(
        exec.commands(),
        vec!["npm ci".to_string(), "npm run checks:code".to_string()],
    );
}
