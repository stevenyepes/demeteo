//! One strict [`ExecutionPort`] double, shared by the step executor's test
//! files.
//!
//! It **errors on anything it was not explicitly told to answer.** AGENTS.md §7
//! names the opposite shape — the e2e `FakeExec` answering `Ok("")` for every
//! command — as what makes a suite unable to fail: a gate asserted against a
//! default is asserted against nothing, and everything *reading* a command's
//! output is then asserted against an empty string it never received.
//!
//! It records `(command, ShellOptions)` in call order rather than the command
//! alone, because two of the properties under test are about the options and
//! not the string: each harness gets **its own** deadline (HB5/S10), and the
//! settings-time probe names **no** working directory (HB6).
//!
//! Mounted exactly once, from `adapters/step_executor/mod.rs`, on the precedent
//! of `adapters/agent/test_stubs.rs`: three `#[path]` mounts of one file load it
//! into the crate graph three times and trip `clippy::duplicate-mod`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use crate::ports::execution::{ExecutionPort, ShellOptions};

pub(crate) struct ScriptedExec {
    answers: HashMap<String, Result<String, String>>,
    files: HashMap<String, Result<String, String>>,
    seen: Mutex<Vec<(String, ShellOptions)>>,
}

fn script(entries: &[(&str, Result<&str, &str>)]) -> HashMap<String, Result<String, String>> {
    entries
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                match v {
                    Ok(s) => Ok(s.to_string()),
                    Err(e) => Err(e.to_string()),
                },
            )
        })
        .collect()
}

impl ScriptedExec {
    pub(crate) fn new(answers: &[(&str, Result<&str, &str>)]) -> Self {
        Self {
            answers: script(answers),
            files: HashMap::new(),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Script `read_file` answers by absolute path. An unscripted path still
    /// errors, so "the adapter read a file it was never told about" is a
    /// failure rather than an empty string.
    pub(crate) fn with_files(mut self, files: &[(&str, Result<&str, &str>)]) -> Self {
        self.files = script(files);
        self
    }

    /// Rewrite every scripted key through `f`, so a test can script the answer
    /// against the command it *authored* rather than the command the adapter is
    /// handed — the baseline's `( … ) 2>&1` wrap being the case that needs it.
    pub(crate) fn map_keys(self, f: impl Fn(&str) -> String) -> Self {
        Self {
            answers: self.answers.into_iter().map(|(k, v)| (f(&k), v)).collect(),
            files: self.files,
            seen: self.seen,
        }
    }

    pub(crate) fn commands(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(c, _)| c.clone())
            .collect()
    }

    pub(crate) fn timeouts(&self) -> Vec<Option<Duration>> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, o)| o.timeout)
            .collect()
    }

    pub(crate) fn options(&self) -> Vec<ShellOptions> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, o)| o.clone())
            .collect()
    }
}

#[async_trait::async_trait]
impl ExecutionPort for ScriptedExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command_with(
        &self,
        _m: &str,
        cmd: &str,
        o: ShellOptions,
    ) -> Result<String, String> {
        self.seen.lock().unwrap().push((cmd.to_string(), o));
        self.answers
            .get(cmd)
            .cloned()
            .unwrap_or_else(|| Err(format!("ScriptedExec: unscripted command `{cmd}`")))
    }
    async fn read_file(&self, _m: &str, p: &str) -> Result<String, String> {
        self.files
            .get(p)
            .cloned()
            .unwrap_or_else(|| Err(format!("ScriptedExec: unscripted read_file `{p}`")))
    }
    async fn write_file(&self, _m: &str, _p: &str, _c: &str) -> Result<(), String> {
        Err("unscripted write_file".into())
    }
    async fn write_file_bytes(&self, _m: &str, _p: &str, _c: &[u8]) -> Result<(), String> {
        Err("unscripted write_file_bytes".into())
    }
    async fn get_metadata(
        &self,
        _m: &str,
        _p: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        Err("unscripted get_metadata".into())
    }
    async fn list_dir(
        &self,
        _m: &str,
        _p: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        Err("unscripted list_dir".into())
    }
    async fn setup_worktree(&self, _m: &str, _r: &str, _b: &str, _s: &str) -> Result<(), String> {
        Err("unscripted setup_worktree".into())
    }
    async fn resolve_home(&self, _m: &str) -> Result<String, String> {
        Err("unscripted resolve_home".into())
    }
    async fn resolve_user(&self, _m: &str) -> Result<String, String> {
        Err("unscripted resolve_user".into())
    }
    async fn resolve_platform(&self, _m: &str) -> Result<crate::domain::models::Platform, String> {
        Err("unscripted resolve_platform".into())
    }
    async fn control_rpc(
        &self,
        _m: &str,
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("unscripted control_rpc".into())
    }
    fn spawn_interactive(
        &self,
        _m: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("unscripted spawn_interactive".into())
    }
}
