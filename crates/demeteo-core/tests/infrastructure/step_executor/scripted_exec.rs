// Tests for `src/adapters/step_executor/scripted_exec.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// The double is the oracle for every step-executor test, so the two things
// worth pinning are that it records the *whole* request — env included, which
// the rendered argv drops — and that it still refuses what it was not told.

use super::*;
use std::collections::BTreeMap;

fn request(executable: &str, args: &[&str], env: &[(&str, &str)]) -> ProgramRequest {
    ProgramRequest {
        executable: executable.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        ..ProgramRequest::default()
    }
}

#[tokio::test]
async fn requests_retain_the_env_the_rendered_argv_drops() {
    let exec = ScriptedExec::new(&[]).with_programs(&[("git status", Ok("clean"))]);
    let sent = request("git", &["status"], &[("GIT_TERMINAL_PROMPT", "0")]);

    let out = exec.run_program("m", sent.clone()).await;

    assert_eq!(out, Ok("clean".to_string()));
    assert_eq!(exec.requests(), vec![sent]);
    assert_eq!(exec.programs(), vec!["git status".to_string()]);
    assert_eq!(exec.calls(), vec!["git status".to_string()]);
}

#[tokio::test]
async fn an_unscripted_program_still_errors_and_is_still_recorded() {
    let exec = ScriptedExec::new(&[]).with_programs(&[("git status", Ok("clean"))]);
    let sent = request("git", &["push"], &[("K", "V")]);

    let out = exec.run_program("m", sent.clone()).await;

    assert_eq!(
        out,
        Err("ScriptedExec: unscripted program `git push`".to_string())
    );
    assert_eq!(exec.requests(), vec![sent]);
}
