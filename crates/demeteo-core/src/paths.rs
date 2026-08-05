//! Centralised helpers for computing the on-disk paths Demeteo uses to
//! store project state and cloned repositories.
//!
//! Two motivations drove the extraction into a single module:
//!
//! 1. **Single source of truth.** The bootstrap, workspace health check,
//!    and step executor all need the *same* target directory for a given
//!    project. A divergence here produces the classic "workspace says
//!    CLONED but the agent can't find the dir" failure where the health
//!    check probes one path and the agent `cd`s into another.
//! 2. **No `~` expansion in path construction.** Previously the codebase
//!    computed paths like `~/.demeteo/projects/<id>/repos/<name>` and
//!    relied on bash to expand `~` inside the SSH command. That works
//!    most of the time, but it ties us to the remote shell's expansion
//!    rules. If HOME is unset, the user has been renamed, or the remote
//!    is configured with a non-standard passwd entry, the agent's
//!    `cd ~/.demeteo/...` will silently land in the wrong place. We
//!    now resolve HOME once via [`ExecutionPort::resolve_home`] and use
//!    the absolute path everywhere.
//!
//! This module is also the single source of truth for small primitives
//! shared by many callers:
//!
//! * [`shell_escape_posix`] — single-quote-escape a string for safe
//!   inclusion in a POSIX shell command. Was duplicated in 5 files.
//! * [`now_ms`] / [`now_secs`] — monotonic timestamp helpers used by
//!   the database adapters, intercept payload, and command handlers.
//! * [`new_id`] — short hex ID built from the wall clock and the
//!   current thread id. Collision-resistant enough for in-app IDs.
//!
//! All public path functions take an [`ExecutionPort`] (so the remote
//! HOME can be resolved) and return absolute paths.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ports::execution::ExecutionPort;

/// The subdirectory of the user's HOME in which Demeteo stores all
/// project state. Kept under a single hidden directory so a `rm -rf`
/// from the user can't accidentally nuke it, and so a single
/// `du -sh ~/.demeteo` answers "how much disk is Demeteo using?".
pub const DEMETEO_HOME_SUBDIR: &str = ".demeteo";

/// The subdirectory under [`DEMETEO_HOME_SUBDIR`] where individual
/// projects live.
pub const PROJECTS_SUBDIR: &str = "projects";

/// The subdirectory under each project where the cloned repository
/// working trees live.
pub const REPOS_SUBDIR: &str = "repos";

/// The subdirectory under each project holding the linked worktrees an
/// interactive terminal session owns, one directory per repository.
///
/// A sibling of [`REPOS_SUBDIR`] rather than a child of it, for the reason
/// recorded on `terminal_worktree_area` in
/// `crates/demeteo-core/src/adapters/worktree/git_ops/worktree.rs`. Spelled
/// here so the adapter that creates them and the application filter that
/// recognises them cannot drift apart.
pub const TERMINAL_WORKTREES_SUBDIR: &str = "terminal-worktrees";

/// Resolve the Demeteo project root for `project_id` on the target host.
///
/// For local projects this is `<home>/.demeteo/projects/<project_id>`.
/// For remote projects this is the remote HOME + the same suffix, with
/// the remote HOME obtained by calling
/// [`ExecutionPort::resolve_home`] so we never depend on `~` expansion
/// in the SSH command.
///
/// `compute_type` is the project's `compute_type` field
/// (`"local"` or `"remote"`); `remote_host` is `Some(<machine_id>)`
/// for remote projects and `None` for local.
/// Resolve the project root for a **local** project.
///
/// `workspace_dir` is the user-configurable base directory (defaults to
/// Tauri's `app_local_data_dir()`). This is a pure, synchronous helper —
/// no shell calls, no identifier hard-coding.
pub fn project_root_local(workspace_dir: &std::path::Path, project_id: &str) -> PathBuf {
    workspace_dir.join("projects").join(project_id)
}

/// Resolve the cloned-repository target dir for a **local** project.
pub fn repo_target_dir_local(
    workspace_dir: &std::path::Path,
    project_id: &str,
    repo_path: &str,
) -> PathBuf {
    project_root_local(workspace_dir, project_id)
        .join(REPOS_SUBDIR)
        .join(repo_name_from_path(repo_path))
}

/// Resolve the project root on the target host.
///
/// For local projects, pass `workspace_dir: Some(base)` — the base comes
/// from Tauri's `app_local_data_dir()` (or a user override) so no
/// identifier is ever hard-coded here. For remote projects, pass
/// `workspace_dir: None`; the root is resolved via SSH `$HOME`.
pub async fn project_root(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
    workspace_dir: Option<&std::path::Path>,
) -> Result<PathBuf, String> {
    if compute_type.eq_ignore_ascii_case("local") {
        let base = workspace_dir.ok_or_else(|| {
            "workspace_dir is required when resolving a local project root".to_string()
        })?;
        Ok(project_root_local(base, project_id))
    } else {
        let home = resolve_home(exec, compute_type, remote_host).await?;
        Ok(home
            .join(DEMETEO_HOME_SUBDIR)
            .join(PROJECTS_SUBDIR)
            .join(project_id))
    }
}

/// Resolve the absolute path of a cloned repository's working tree.
///
/// For a project with `id = p1781624953648` and `repo_path =
/// "prototype/spectacular"`, this returns
/// `<home>/.demeteo/projects/p1781624953648/repos/spectacular`.
///
/// The returned path is absolute and contains no `~`, so it's safe to
/// pass to `git -C`, `cd`, or SFTP calls without further shell
/// expansion.
pub async fn repo_target_dir(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
    repo_path: &str,
    workspace_dir: Option<&std::path::Path>,
) -> Result<PathBuf, String> {
    if compute_type.eq_ignore_ascii_case("local") {
        let base = workspace_dir.ok_or_else(|| {
            "workspace_dir is required when resolving a local repo target dir".to_string()
        })?;
        Ok(repo_target_dir_local(base, project_id, repo_path))
    } else {
        Ok(
            project_root(exec, compute_type, remote_host, project_id, None)
                .await?
                .join(REPOS_SUBDIR)
                .join(repo_name_from_path(repo_path)),
        )
    }
}

/// Same as [`repo_target_dir`] but returns a `String` (the form most
/// existing callers want when building shell commands).
pub async fn repo_target_dir_str(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
    project_id: &str,
    repo_path: &str,
    workspace_dir: Option<&std::path::Path>,
) -> Result<String, String> {
    repo_target_dir(
        exec,
        compute_type,
        remote_host,
        project_id,
        repo_path,
        workspace_dir,
    )
    .await
    .map(|p| p.to_string_lossy().to_string())
}

/// Whether an operation aimed at `machine_id` lands on a Windows filesystem.
///
/// `host_is_windows` is the caller's `cfg!(windows)`, passed rather than read so
/// the two answers are both reachable from a test on either platform.
///
/// A remote machine is Linux (R2, `docs/REMOTE_EXECUTION.md`), so only the local
/// machine can be the Windows one — a Windows desktop driving a remote must
/// still emit the POSIX form. Anything that branches on "is this Windows" needs
/// *both* halves; branching on `cfg!(windows)` alone is the shape that sends a
/// `chmod` to Linux or a `MAX_PATH` workaround to a machine with no `MAX_PATH`.
pub fn windows_host_target(host_is_windows: bool, machine_id: &str) -> bool {
    host_is_windows && crate::domain::ids::MachineId::from(machine_id).is_local()
}

/// [`windows_host_target`] against this build's platform.
pub fn targets_windows_host(machine_id: &str) -> bool {
    windows_host_target(cfg!(windows), machine_id)
}

/// `path` as the host that owns it spells it, whoever reported it.
///
/// # One directory, three spellings
///
/// On Windows the same location reaches Demeteo under three names, from three
/// producers: `C:\Users\…` from a [`PathBuf`], `C:/Users/…` from git, which
/// reports forward slashes on every platform, and `/c/Users/…` from anything
/// that has been through Git Bash — `pwd` inside a shell answers in that MSYS
/// form, and `git_ops::worktree`'s terminal-worktree creation reads a path out
/// of one.
///
/// The MSYS form is not merely a third spelling to tolerate. No Win32 call
/// accepts it, so it is a path that names nothing: a terminal opened there
/// fails, and so does every `std::fs` call. It is converted here, at the
/// boundary it enters through, rather than tolerated at each comparison
/// downstream.
///
/// `windows_host` is the caller's [`windows_host_target`] answer, passed rather
/// than read so both halves are reachable from a test on either platform. It
/// must be false for a remote target even on a Windows desktop: there
/// `/c/anything` is an ordinary directory and rewriting it would invent a drive.
pub fn native_path(path: &str, windows_host: bool) -> PathBuf {
    if !windows_host {
        return PathBuf::from(path);
    }
    let drive = match path.as_bytes() {
        [b'/', drive, rest @ ..]
            if drive.is_ascii_alphabetic() && matches!(rest, [] | [b'/', ..]) =>
        {
            Some(drive.to_ascii_uppercase() as char)
        }
        _ => None,
    };
    let spelled = match drive {
        Some(drive) => format!("{drive}:\\{}", path[2..].trim_start_matches('/')),
        None => path.to_string(),
    };
    PathBuf::from(spelled.replace('/', "\\"))
}

/// Whether `path` is absolute under the rules of the host that owns it.
///
/// [`std::path::Path::is_absolute`] answers for the platform this was
/// *compiled* for, which is the wrong question for every path belonging to
/// another host. A Windows desktop driving a Linux machine reads `/srv/…` back
/// from it; `Path::is_absolute` calls that relative, because Windows wants a
/// drive letter or a UNC prefix — so a caller filtering on it silently discards
/// the one answer the target gave, and falls back to whatever it guessed.
///
/// `windows_host` is the caller's [`windows_host_target`] answer, passed rather
/// than read so both answers are reachable from a test on either platform.
pub fn is_absolute_on(path: &str, windows_host: bool) -> bool {
    fn separator(byte: u8) -> bool {
        byte == b'/' || byte == b'\\'
    }
    if !windows_host {
        return path.starts_with('/');
    }
    let bytes = path.as_bytes();
    matches!(bytes, [first, second, ..] if separator(*first) && separator(*second))
        || matches!(bytes, [drive, b':', sep, ..] if drive.is_ascii_alphabetic() && separator(*sep))
}

/// Whether `path` reads as absolute under *either* platform's rules.
///
/// For paths recovered from text whose producing host is not knowable where it
/// is read: one path manifest carries artifact paths written by the desktop
/// alongside worktree paths belonging to the machine the step runs on, so
/// neither platform's rules alone recognise all of them. What the path *is* —
/// a file here, a directory over there — is then settled by asking, not by the
/// spelling.
pub fn looks_absolute(path: &str) -> bool {
    is_absolute_on(path, false) || is_absolute_on(path, true)
}

/// `base` with `components` appended, spelled with the separator the host that
/// owns `base` uses.
///
/// [`std::path::Path::join`] writes the separator of the platform this was
/// *compiled* for, so a Windows desktop composing a path inside a Linux
/// worktree produces `/home/u/wt\artifacts\_context` — a single directory whose
/// name contains backslashes, which SFTP creates without complaint and no later
/// lookup ever finds again.
///
/// `windows_host` is the caller's [`windows_host_target`] answer.
pub fn join_on<'a>(
    base: &str,
    components: impl IntoIterator<Item = &'a str>,
    windows_host: bool,
) -> String {
    let separator = if windows_host { '\\' } else { '/' };
    let mut joined = base.trim_end_matches(['/', '\\']).to_string();
    for component in components {
        joined.push(separator);
        joined.push_str(component);
    }
    joined
}

/// Whether two spellings name one location.
///
/// Every caller of this is comparing a path git reported against one Demeteo
/// built, so both go through [`native_path`] first and then through
/// [`std::path::Path`]'s component equality — which a string comparison gets
/// wrong at a trailing separator and at a doubled one.
///
/// Case-insensitive on a Windows host, as NTFS is. A drive letter that arrived
/// lowercased — every MSYS path does — must not fork one directory into two.
pub fn same_path(a: &str, b: &str, windows_host: bool) -> bool {
    comparison_key(a, windows_host) == comparison_key(b, windows_host)
}

fn comparison_key(path: &str, windows_host: bool) -> PathBuf {
    let native = native_path(path, windows_host);
    if !windows_host {
        return native;
    }
    // A trailing separator is folded away by [`std::path::Path`] only on a
    // Windows *build*, and `windows_host` is a parameter precisely so the
    // Windows answer stays reachable from a Linux test — so it cannot be left
    // to whichever `Path` this was compiled against.
    let mut key = native.to_string_lossy().to_ascii_lowercase();
    while key.len() > 3 && key.ends_with('\\') {
        key.pop();
    }
    PathBuf::from(key)
}

/// How many hex digits [`short_path_segment`] emits.
pub const SHORT_SEGMENT_LEN: usize = 8;

/// Fold an identifier into a fixed 8 hex digits for use as a path segment.
///
/// # Why not a prefix of the id
///
/// Demeteo's ids are `<tag><wall-clock millis>` (`p1781624953648`,
/// `f-1781624953648-step-s-implement`). The first eight characters of that are
/// the *high* digits of the timestamp, which change once every ~16 minutes — so
/// a literal prefix collides between any two entities created in the same
/// afternoon, and a collision here is two features sharing one worktree
/// directory, which `test_provision_subtask_worktree_same_repo_two_features_do_not_collide`
/// records as data loss rather than a name clash. The distinguishing bits are in
/// the tail, so the whole id has to be read to keep them.
///
/// # Why FNV-1a rather than `DefaultHasher`
///
/// [`new_id`] can use `DefaultHasher` because its output only has to be unique
/// within one process. This one names a directory that outlives the process: a
/// worktree provisioned by one build is torn down by whichever build is running
/// when the step ends, and `DefaultHasher`'s output is explicitly not stable
/// across Rust releases. A stale directory nobody can name again is a leak that
/// only a rebuild produces, which is the worst kind to find.
pub fn short_path_segment(id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
}

/// Extract the repository name (last `/`-separated segment) from a
/// `repo_path` like `"prototype/spectacular"`.
pub fn repo_name_from_path(repo_path: &str) -> String {
    repo_path
        .split('/')
        .rfind(|s| !s.is_empty())
        .unwrap_or(repo_path)
        .to_string()
}

/// Resolve the absolute home directory for the target host.
///
/// The implementation just delegates to
/// [`ExecutionPort::resolve_home`]; the wrapper exists so the
/// `local` / `remote` discrimination lives in one place.
async fn resolve_home(
    exec: &Arc<dyn ExecutionPort>,
    compute_type: &str,
    remote_host: Option<&str>,
) -> Result<PathBuf, String> {
    let machine_id = if compute_type.eq_ignore_ascii_case("local") {
        "local"
    } else {
        remote_host.ok_or_else(|| {
            "Remote project has no `remote_host` set; cannot resolve HOME".to_string()
        })?
    };
    let home_str = exec
        .resolve_home(machine_id)
        .await
        .map_err(|e| format!("Failed to resolve HOME on '{}': {}", machine_id, e))?;
    Ok(PathBuf::from(home_str))
}

#[cfg(test)]
#[path = "../tests/unit/paths/tests.rs"]
mod tests;

// ─────────────────────────────────────────────────────────────────────────────
// Shared primitives (shell escaping, time, IDs)
//
// These were previously duplicated across `commands/project.rs`,
// `commands/bootstrap.rs`, `commands/workflows.rs`, `commands/providers.rs`,
// `adapters/ssh/client.rs`, `adapters/step_executor/mod.rs`,
// `adapters/worktree/git_ops/`, and `domain/intercept.rs`. Each duplicate
// was a near-copy of the same algorithm; a single change to the escape
// strategy (e.g. switch to `printf %q`) used to require touching 5 files.
// The new canonical home is `crate::shared::*`; this module keeps the
// legacy function names verbatim so the migration is incremental. New
// code should prefer `crate::shared::*`.
// ─────────────────────────────────────────────────────────────────────────────

/// Single-quote-escape `s` for safe inclusion in a POSIX shell command.
///
/// * `~` and `~/...` pass through unchanged so the remote shell expands them.
/// * Strings made entirely of "safe" characters (alnum + `_-. /=:,@`) are
///   returned verbatim (the fast path; matches the previous local behaviour).
/// * Everything else is wrapped in single quotes with internal `'` escaped
///   via the standard `'\''` trick.
///
/// This is the only POSIX shell escaper in the codebase. If you find
/// yourself reaching for `format!("... {}", something)` to build a shell
/// command, route the `something` through this function.
pub fn shell_escape_posix(s: &str) -> String {
    crate::shared::shell::escape_posix(s)
}

/// A `git -C <dir>` prefix for the commits Demeteo makes on its own behalf,
/// with the target repo's hooks disabled.
///
/// Demeteo commits for its own bookkeeping — subtask merges, conflict
/// resolutions, artifact snapshots — with machine-generated messages on
/// pipeline-owned branches. Target repos routinely install a `commit-msg`
/// hook (husky + commitlint) or a `pre-commit` hook that runs the full test
/// suite, and those reject our messages outright:
///
/// ```text
/// ✖ subject may not be empty [subject-empty]
/// Not committing merge; use 'git commit' to complete the merge.
/// ```
///
/// A rejected merge commit is deterministic, so the retry hits the identical
/// hook and the pipeline can never make progress. Hooks still run for the
/// agent's own code commits inside the worktree, which is where a repo's
/// lint/test gates actually belong.
///
/// Pointing `core.hooksPath` at a directory that cannot contain hooks
/// disables every hook on every git version — unlike `--no-verify`, which
/// git only honours for `merge` as of 2.36 and which skips only some hooks.
pub fn git_no_hooks(dir: &str) -> String {
    format!(
        "git -c core.hooksPath=/dev/null -C {}",
        shell_escape_posix(dir)
    )
}

/// Well-known gitignored dependency directories that a fresh
/// `git worktree add` doesn't carry over — they're gitignored, so no
/// commit brings their contents along, and a bare worktree checkout
/// leaves them empty. Build/test harnesses (`npm test`, `cargo test`,
/// `pytest`) fail immediately without them.
///
/// # These are build output, and build output is per-feature
///
/// Every entry here is **mutable, per-branch state** — the result of
/// installing or compiling *this* branch's code. None of them is a
/// content-addressed download cache. (Those — the Cargo registry, npm's
/// `_cacache`, the pip wheel cache — live outside the repo in `~/.cargo`,
/// `~/.npm`, and are immutable-by-content, so sharing them across features is
/// both safe and where the real time saving is. Nothing here touches them.)
///
/// A project runs N features concurrently ([`DECISIONS.md`] decision 18), so
/// these must never be shared *between* features. They used to be: every
/// worktree symlinked straight to `{repo}/node_modules`, so feature B's
/// install silently overwrote feature A's. The damage was not merely a
/// corrupted tree — a `verify` step's harness verdict could be decided by
/// another feature's build output, and that verdict drives Demeteo's retry and
/// critic loops. A feature would chase a failure that belonged to someone else.
///
/// So each *feature* gets its own cache root ([`feature_cache_dir`]), seeded
/// once from the primary checkout, and every worktree of that feature symlinks
/// into it. Steps within one feature are sequential, so sharing across a
/// feature's own worktrees is safe.
///
/// Anything added to this list must be classified against that rule first:
/// **share content-addressed download caches; never share build output.**
///
/// Important: a symlink standing in for a directory is NOT recognized
/// by git as matching a trailing-slash `.gitignore` pattern (e.g.
/// `node_modules/` matches a real directory but not a symlink named
/// `node_modules`), so a linked cache shows up as untracked and, left
/// alone, an absolute host path gets committed onto the feature branch.
/// The answer is a **slashless entry in the clone's own
/// `.git/info/exclude`**, written by `git_ops::worktree` before any link
/// is made — `node_modules` without the slash matches a symlink and a
/// directory alike, so from `git add -A`'s point of view the entry is
/// simply ignored and no pathspec is involved. Doing it there rather
/// than at `git add` time is what keeps the answer the same on a
/// platform that shares no caches at all: nothing is linked, nothing is
/// excluded, and the same feature captures the same files.
///
/// [`DECISIONS.md`]: https://github.com/stevenyepes/demeteo/blob/master/docs/DECISIONS.md
pub const DEPENDENCY_CACHE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    ".next",
    "vendor",
    ".tox",
    "__pycache__",
];

/// The dependency-cache root owned by one feature: `{repo_dir}_cache_{branch}`.
///
/// Keyed by the feature branch because that is the one identifier
/// `provision_subtask_worktree` already has that is unique per feature *and*
/// stable across the feature's steps — each step gets a fresh worktree, but
/// they all belong to one feature and may share its cache.
///
/// Sits alongside the repo rather than inside it, exactly as the worktree dirs
/// (`{repo_dir}_wt_{subtask_id}`) do, so it is never a candidate for `git add`
/// and never collides with a path the agent can write.
pub fn feature_cache_dir(repo_dir: &str, feature_branch: &str) -> String {
    // `feature/foo` → `feature-foo`: the branch is a path component here, and a
    // slash would silently nest the cache under a `feature/` directory.
    let slug: String = feature_branch
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect();
    format!("{}_cache_{}", repo_dir, slug)
}

/// Current wall-clock time in milliseconds since the UNIX epoch.
///
/// Used for `created_at` / `updated_at` columns, sidebar ordering, and
/// ad-hoc timing in workflow command handlers. The single source of truth
/// (was duplicated in 4 files).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Current wall-clock time in seconds since the UNIX epoch. Used by
/// `domain/intercept.rs` to build the `created_at` field on the
/// `permission_requested` payload.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Generate a short, unique-enough identifier for in-app entities
/// (workflow rows, step configurations, intercepted commands).
///
/// Not cryptographically random — it's a `DefaultHasher` of the wall
/// clock and the current thread id, formatted as 16 hex digits. Good
/// enough for the "no two rows in the same table share an id" property
/// inside one process; **not** suitable for security tokens.
pub fn new_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    std::thread::current().id().hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
#[path = "../tests/unit/paths/primitive_tests.rs"]
mod primitive_tests;
