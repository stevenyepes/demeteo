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

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use crate::ports::execution::{ExecutionPort, ProgramRequest, ShellOptions};

pub(crate) struct ScriptedExec {
    answers: HashMap<String, Result<String, String>>,
    /// Answers consumed in call order for one command, for the reads whose
    /// whole subject is that git's answer *changed* — the same
    /// `status --porcelain` before and after a resolution. A queue that runs
    /// out errors rather than falling back, so "it asked once more than the
    /// test anticipated" stays a failure.
    queued: Mutex<HashMap<String, Vec<Result<String, String>>>>,
    files: HashMap<String, Result<String, String>>,
    /// The same idea as [`ScriptedExec::queued`], one port over: a conflict
    /// resolution reads each conflicted file before its turn and again after
    /// it, and the whole subject of the loop between them is that the second
    /// read says something different. A single answer per path can only
    /// describe a tree that never moved.
    queued_files: Mutex<HashMap<String, Vec<Result<String, String>>>>,
    programs: HashMap<String, Result<String, String>>,
    dirs: HashSet<String>,
    /// Watches to trip as a command is issued, which is the one shape a
    /// scripted *answer* cannot produce: a stop that arrives while a command
    /// is already in flight.
    stops: HashMap<String, tokio::sync::watch::Sender<bool>>,
    seen: Mutex<Vec<(String, ShellOptions)>>,
    seen_programs: Mutex<Vec<String>>,
    /// Both of the above, interleaved in the order they actually happened.
    ///
    /// The two recorders above are separate so a test over shell commands is
    /// not perturbed by git plumbing beside it — but that leaves an ordering
    /// property spanning both ports invisible in either. A sync resolution
    /// commits through `run_command` and pushes through `run_program`, and
    /// "committed before published" is exactly such a property: with only the
    /// two lists it reads as a push that never happened.
    seen_all: Mutex<Vec<String>>,
}

/// The argv of a [`ProgramRequest`] joined by single spaces — the form a test
/// scripts and asserts against. Logged separately from
/// [`ScriptedExec::commands`] so a test over shell commands is not perturbed
/// by whatever git plumbing ran beside them.
fn rendered(request: &ProgramRequest) -> String {
    std::iter::once(request.executable.as_str())
        .chain(request.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
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
            queued: Mutex::new(HashMap::new()),
            files: HashMap::new(),
            queued_files: Mutex::new(HashMap::new()),
            programs: HashMap::new(),
            dirs: HashSet::new(),
            stops: HashMap::new(),
            seen: Mutex::new(Vec::new()),
            seen_programs: Mutex::new(Vec::new()),
            seen_all: Mutex::new(Vec::new()),
        }
    }

    /// Script successive answers for one command, consumed in call order.
    pub(crate) fn with_queue(mut self, cmd: &str, answers: &[Result<&str, &str>]) -> Self {
        let queue = answers
            .iter()
            .rev()
            .map(|v| match v {
                Ok(s) => Ok(s.to_string()),
                Err(e) => Err(e.to_string()),
            })
            .collect();
        self.queued
            .get_mut()
            .unwrap()
            .insert(cmd.to_string(), queue);
        self
    }

    /// Script [`ExecutionPort::run_program`] answers, keyed by [`rendered`]
    /// argv. An unscripted program still errors.
    pub(crate) fn with_programs(mut self, programs: &[(&str, Result<&str, &str>)]) -> Self {
        self.programs = script(programs);
        self
    }

    /// Send `true` on `tx` when `cmd` is issued, before answering it.
    ///
    /// The yield below is load-bearing rather than tidiness:
    /// [`run_harness_command`](crate::adapters::step_executor::harness_shell::run_harness_command)
    /// races the run future against the watch, and a future that is `Ready` on
    /// its first poll is never raced at all.
    pub(crate) fn with_stop_on(mut self, cmd: &str, tx: tokio::sync::watch::Sender<bool>) -> Self {
        self.stops.insert(cmd.to_string(), tx);
        self
    }

    /// Declare which absolute paths `get_metadata` reports as directories.
    /// Every other path stays an error, so "the code probed somewhere this
    /// test never set up" fails rather than reading as an empty disk.
    pub(crate) fn with_dirs(mut self, dirs: &[&str]) -> Self {
        self.dirs = dirs.iter().map(|d| d.to_string()).collect();
        self
    }

    /// Every `run_program` this double was handed, in call order.
    pub(crate) fn programs(&self) -> Vec<String> {
        self.seen_programs.lock().unwrap().clone()
    }

    /// Script `read_file` answers by absolute path. An unscripted path still
    /// errors, so "the adapter read a file it was never told about" is a
    /// failure rather than an empty string.
    pub(crate) fn with_files(mut self, files: &[(&str, Result<&str, &str>)]) -> Self {
        self.files = script(files);
        self
    }

    /// Script successive `read_file` answers for one path, consumed in call
    /// order, the last of which then stands.
    ///
    /// Where [`Self::with_queue`] errors once exhausted, this holds — because a
    /// file and a command differ in what a repeated ask means. Asking git the
    /// same question twice can legitimately want two answers, so an unplanned
    /// third ask is a test that lost track of its subject. A file read twice
    /// with no write between them can only say the same thing, and an extra
    /// read is a caller being careful, not a caller being wrong.
    pub(crate) fn with_queued_file(self, path: &str, answers: &[Result<&str, &str>]) -> Self {
        let queue = answers
            .iter()
            .rev()
            .map(|v| match v {
                Ok(s) => Ok(s.to_string()),
                Err(e) => Err(e.to_string()),
            })
            .collect();
        self.queued_files
            .lock()
            .unwrap()
            .insert(path.to_string(), queue);
        self
    }

    /// Rewrite every scripted key through `f`, so a test can script the answer
    /// against the command it *authored* rather than the command the adapter is
    /// handed — the baseline's `( … ) 2>&1` wrap being the case that needs it.
    pub(crate) fn map_keys(self, f: impl Fn(&str) -> String) -> Self {
        Self {
            answers: self.answers.into_iter().map(|(k, v)| (f(&k), v)).collect(),
            queued: self.queued,
            files: self.files,
            queued_files: self.queued_files,
            programs: self.programs,
            dirs: self.dirs,
            stops: self.stops,
            seen: self.seen,
            seen_programs: self.seen_programs,
            seen_all: self.seen_all,
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

    /// Every command and every program, in one order — see `seen_all`.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.seen_all.lock().unwrap().clone()
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
    async fn run_program(&self, _m: &str, request: ProgramRequest) -> Result<String, String> {
        let key = rendered(&request);
        self.seen_programs.lock().unwrap().push(key.clone());
        self.seen_all.lock().unwrap().push(key.clone());
        self.programs
            .get(&key)
            .cloned()
            .unwrap_or_else(|| Err(format!("ScriptedExec: unscripted program `{key}`")))
    }
    async fn run_command_with(
        &self,
        _m: &str,
        cmd: &str,
        o: ShellOptions,
    ) -> Result<String, String> {
        self.seen.lock().unwrap().push((cmd.to_string(), o));
        self.seen_all.lock().unwrap().push(cmd.to_string());
        if let Some(tx) = self.stops.get(cmd) {
            let _ = tx.send(true);
            tokio::task::yield_now().await;
        }
        if let Some(queue) = self.queued.lock().unwrap().get_mut(cmd) {
            return queue
                .pop()
                .unwrap_or_else(|| Err(format!("ScriptedExec: queue exhausted for `{cmd}`")));
        }
        self.answers
            .get(cmd)
            .cloned()
            .unwrap_or_else(|| Err(format!("ScriptedExec: unscripted command `{cmd}`")))
    }
    async fn read_file(&self, _m: &str, p: &str) -> Result<String, String> {
        if let Some(queue) = self.queued_files.lock().unwrap().get_mut(p) {
            if queue.len() > 1 {
                if let Some(answer) = queue.pop() {
                    return answer;
                }
            }
            if let Some(last) = queue.last() {
                return last.clone();
            }
        }
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
        p: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        if self.dirs.contains(p) {
            return Ok(crate::ports::execution::SftpEntry {
                name: p.to_string(),
                path: p.to_string(),
                is_dir: true,
                size: 0,
                modified: 0,
            });
        }
        Err(format!("ScriptedExec: unscripted get_metadata `{p}`"))
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
