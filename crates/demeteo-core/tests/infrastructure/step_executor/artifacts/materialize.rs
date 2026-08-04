//! Where the bytes actually land — asserted against the machine id, not the
//! host filesystem.

use super::*;

// ── materialize_external_artifact_paths (remote-machine regression) ─────
//
// The previous implementation used `std::fs::copy` /
// `std::fs::create_dir_all` unconditionally, which silently dropped
// bytes for remote steps: the worktree path string pointed at a
// directory on the SSH target, not on the Tauri host, so the local
// `std::fs` calls failed (or wrote a phantom file to a path that
// didn't exist remotely), and the opencode agent on the remote box
// ended up with a prompt pointing at a file it couldn't `Read` under
// its `external_directory: deny` fence.
//
// The fix routes the write through `ExecutionPort::write_file`, which
// dispatches SFTP for remote and `std::fs` for local. These tests
// pin both halves of that contract:

use std::sync::Mutex;

/// `ExecutionPort` double that records every `write_file` /
/// `write_file_bytes` / `get_metadata` / `create_dir_all` call so the test can
/// assert the artifact ended up on the *target* host (path string) — not on the
/// Tauri host.
///
/// It answers **no** shell command at all: the destination directory is made
/// with `create_dir_all`, and a `mkdir -p` reaching this double is the
/// regression that would put the shell back. A double that answers everything
/// with `Ok("")` asserts against a default rather than an answer (AGENTS.md §7,
/// the e2e `FakeExec`).
struct RecordingExec {
    writes: Mutex<Vec<(String, String, String)>>, // (machine_id, path, content)
    dirs: Mutex<Vec<(String, String)>>,           // (machine_id, path)
    metadata_results: Mutex<std::collections::HashMap<String, crate::ports::execution::SftpEntry>>,
}

impl RecordingExec {
    fn new() -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            dirs: Mutex::new(Vec::new()),
            metadata_results: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn write_count(&self) -> usize {
        self.writes.lock().unwrap().len()
    }

    fn recorded_writes(&self) -> Vec<(String, String, String)> {
        self.writes.lock().unwrap().clone()
    }

    fn created_dirs(&self) -> Vec<(String, String)> {
        self.dirs.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::ports::execution::ExecutionPort for RecordingExec {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command(&self, _: &str, cmd: &str) -> Result<String, String> {
        panic!("materialize must not run a shell command: `{cmd}`");
    }
    async fn create_dir_all(&self, machine_id: &str, path: &str) -> Result<(), String> {
        self.dirs
            .lock()
            .unwrap()
            .push((machine_id.to_string(), path.to_string()));
        Ok(())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Err("unscripted read_file".into())
    }
    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        self.writes.lock().unwrap().push((
            machine_id.to_string(),
            path.to_string(),
            content.to_string(),
        ));
        Ok(())
    }
    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let s = String::from_utf8_lossy(content).to_string();
        self.writes
            .lock()
            .unwrap()
            .push((machine_id.to_string(), path.to_string(), s));
        Ok(())
    }
    async fn get_metadata(
        &self,
        _: &str,
        path: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        self.metadata_results
            .lock()
            .unwrap()
            .remove(path)
            .ok_or_else(|| format!("not found: {}", path))
    }
    async fn list_dir(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        Ok(vec![])
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        Ok("/tmp".to_string())
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Ok("test".to_string())
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("control_rpc not supported by RecordingExec".to_string())
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("RecordingExec: spawn_interactive not supported".to_string())
    }
}

fn temp_artifact(name: &str, body: &str) -> (tempdir::TempDir, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "demeteo_materialize_test_{}_{}_{}",
        nanos,
        std::process::id(),
        count
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, body).unwrap();
    (tempdir::TempDir::from_path(dir.clone()), file)
}

#[tokio::test]
async fn materialize_external_paths_writes_to_remote_worktree_via_exec() {
    // Source artifact (always local — the FS artifact store lives on
    // the Tauri host).
    let (_src_dir, src_path) = temp_artifact("implementation-plan.md", "# Plan body\n");
    let src_str = src_path.to_string_lossy().to_string();

    // Target worktree path. This path is on the REMOTE machine: it
    // must NOT be touched on the local host. The previous
    // implementation called `std::fs::create_dir_all` on this string,
    // which silently failed (or wrote a phantom local file).
    let remote_wt = "/home/builder/.demeteo/projects/myrepo/myrepo_wt_f-abc-step-s-implement";

    let exec = RecordingExec::new();
    let prompt = format!(
        "=== ATTACHED CONTEXT: s-plan (path manifest) ===\n\
         The following artifacts from step `s-plan` are on disk:\n\n\
         - `{src}`\n\n\
         Use your Read tool to load them on demand...\n================================\n\n\
         You are an implementation engineer...",
        src = src_str
    );

    let rewritten =
        materialize_external_artifact_paths(&prompt, remote_wt, &exec, "m-builder").await;

    // The remote worktree's _context dir got exactly one write,
    // routed via the exec port to the remote machine_id.
    assert_eq!(
        exec.write_count(),
        1,
        "exactly one write expected; got {:?}",
        exec.recorded_writes()
    );
    let (machine_id, dest_path, content) = &exec.recorded_writes()[0];
    assert_eq!(
        machine_id, "m-builder",
        "write must target the remote machine"
    );
    assert!(
        dest_path.starts_with(remote_wt),
        "destination must live under the remote worktree, got {dest_path}"
    );
    assert!(
        dest_path.ends_with("/artifacts/_context/implementation-plan.md"),
        "destination must be the canonical _context/ copy, got {dest_path}"
    );
    assert_eq!(content, "# Plan body\n", "file body must round-trip");

    // The destination directory is made through the port, on the same machine
    // as the write. `mkdir -p` reached the target's shell; `create_dir_all`
    // reaches its filesystem, which is the one thing every transport has.
    let expected_dir = std::path::Path::new(remote_wt)
        .join("artifacts")
        .join("_context")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        exec.created_dirs(),
        vec![("m-builder".to_string(), expected_dir)]
    );

    // The prompt was rewritten to point at the new path so the
    // opencode Read tool finds the file inside the worktree.
    assert!(
        rewritten.contains(dest_path),
        "rewritten prompt must reference the new path; got: {rewritten}"
    );
    assert!(
        !rewritten.contains(&src_str),
        "old local path must be replaced; got: {rewritten}"
    );

    // The phantom local file MUST NOT exist at the remote path string.
    assert!(
        !std::path::Path::new(dest_path).exists(),
        "no file should be created on the host at the remote worktree's path string"
    );

    drop(_src_dir);
}

/// A worktree under a directory with a space is ordinary on macOS and the norm
/// on Windows. `mkdir -p` needed `shell_escape_posix` around it or the shell
/// split the argument in two; the port takes the path as one argument, so the
/// escaping must be *gone* — quotes carried through would become part of the
/// directory's name.
#[tokio::test]
async fn a_worktree_path_with_a_space_reaches_the_port_unescaped() {
    let (_src_dir, src_path) = temp_artifact("plan.md", "# Plan\n");
    let remote_wt = "/home/builder/my projects/repo_wt_x";

    let exec = RecordingExec::new();
    let prompt = format!("- `{}`\n", src_path.to_string_lossy());
    let _ = materialize_external_artifact_paths(&prompt, remote_wt, &exec, "m-builder").await;

    let expected_dir = std::path::Path::new(remote_wt)
        .join("artifacts")
        .join("_context")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        exec.created_dirs(),
        vec![("m-builder".to_string(), expected_dir)]
    );
    drop(_src_dir);
}

#[tokio::test]
async fn materialize_external_paths_noop_when_no_absolute_paths() {
    // Prompt with no backtick-quoted absolute paths → nothing to copy,
    // prompt returned unchanged.
    let exec = RecordingExec::new();
    let prompt = "You are an implementation engineer.\n\nFollow the spec.";
    let remote_wt = "/home/builder/repo_wt_x";
    let rewritten =
        materialize_external_artifact_paths(prompt, remote_wt, &exec, "m-builder").await;
    assert_eq!(rewritten, prompt);
    assert_eq!(exec.write_count(), 0, "no writes expected for empty prompt");
}

#[tokio::test]
async fn materialize_external_paths_skips_paths_inside_worktree() {
    // An absolute path that already sits inside the worktree (e.g.
    // produced by an earlier materialize step) must be left alone —
    // it's already readable under external_directory: deny.
    let exec = RecordingExec::new();
    let inside_wt = "/home/builder/repo_wt_x/artifacts/_context/already-here.md";
    let prompt = format!("- `{inside_wt}`\n\nbody");
    let rewritten =
        materialize_external_artifact_paths(&prompt, "/home/builder/repo_wt_x", &exec, "m-builder")
            .await;
    assert_eq!(
        rewritten, prompt,
        "paths inside the worktree must NOT be rewritten"
    );
    assert_eq!(exec.write_count(), 0);
}

#[tokio::test]
async fn materialize_external_paths_local_machine_routes_through_exec() {
    // Local-machine regression: same machinery, same fix. The path
    // gets to the right place via exec.write_file (which the local
    // adapter implements as std::fs under the hood).
    let (_src_dir, src_path) = temp_artifact("s-implement.md", "## Files\n");
    let src_str = src_path.to_string_lossy().to_string();

    let local_wt = std::env::temp_dir().join(format!(
        "demeteo_mat_local_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&local_wt).unwrap();

    let exec = RecordingExec::new();
    let prompt = format!("- `{src_str}`\n");
    let rewritten =
        materialize_external_artifact_paths(&prompt, &local_wt.to_string_lossy(), &exec, "local")
            .await;

    assert_eq!(exec.write_count(), 1);
    let (machine_id, dest_path, content) = &exec.recorded_writes()[0];
    assert_eq!(machine_id, "local");
    assert!(dest_path.starts_with(&local_wt.to_string_lossy().to_string()));
    assert_eq!(content, "## Files\n");
    assert!(rewritten.contains(dest_path));

    // Clean up the worktree dest.
    let _ = std::fs::remove_dir_all(&local_wt);
    drop(_src_dir);
}

// ── tempdir re-implementation ───────────────────────────────────────────
//
// The workspace tests use a tiny `tempdir` crate (the standalone
// `tempdir` re-export). It isn't on this crate's dev-dependencies
// for the production build, so we inline a 4-line equivalent here so
// the materialize tests don't pull a new dep.
mod tempdir {
    pub struct TempDir(std::path::PathBuf);
    impl TempDir {
        pub fn from_path(p: std::path::PathBuf) -> Self {
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
