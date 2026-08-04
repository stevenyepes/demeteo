//! Which file on this Windows machine is the bash that runs a user's script.
//!
//! Demeteo executes **one POSIX script body** on every platform; only the
//! interpreter's path differs (`docs/WINDOWS_PARITY.md`). On Windows that
//! interpreter is the bash Git for Windows already installs, which is why this
//! is a search and not a dependency.
//!
//! Three wrong answers have each been shipped by somebody, so each is excluded
//! by construction rather than left to review:
//!
//! - **A bare `bash` resolved through `PATH`.** `C:\Windows\System32\bash.exe`
//!   is the WSL launcher, System32 precedes Git in `PATH`, and WSL bash
//!   resolves none of the Windows paths Demeteo hands it. actions/runner #786
//!   and #216 and GitPython's Windows hook bug are the same mistake three
//!   times. No candidate produced here ever comes from searching `PATH` for
//!   `bash`; [`rejection`] refuses one even if a user supplies it by hand.
//! - **`<root>\git-bash.exe`.** A mintty launcher: it detaches and reports no
//!   exit code, so every command would read as having succeeded instantly —
//!   the one failure mode a human-approval-gated orchestrator can least
//!   afford.
//! - **`<root>\usr\bin\bash.exe` in preference to `<root>\bin\bash.exe`.** The
//!   latter arranges the MSYS `PATH` view; the former is raw MSYS2.
//!
//! And one wrong machine: MinGit ships `git.exe` deliberately without bash,
//! and its BusyBox variant ships an `ash` that would answer a `printf ok`
//! probe happily and then fail on the first bash-only construct. Hence the
//! probe [`ShellHost::bash_version`] specifies, whose `none` answer only a
//! non-bash shell can produce.
//!
//! ## The split
//!
//! [`ShellSearch`] is everything the outside world contributes, gathered once;
//! [`ShellHost`] is the two I/O operations that cannot be gathered in advance.
//! [`resolve`] is then a pure function of the two, and so are the pieces it is
//! built from. Their real implementations — the registry read and the probe
//! spawn — are the whole of `shared/win/discovery.rs`, which is small enough
//! to review by eye because nothing that decides anything is in it.

use std::path::{Path, PathBuf};

/// The interpreter pair one Git for Windows installation provides.
///
/// `sh` degrades to `bash` when the installation carries no `sh.exe`: the
/// bodies Demeteo composes are valid under either, and refusing to run at all
/// over a missing `sh.exe` would be a harsher verdict than the difference
/// deserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PosixShell {
    pub root: PathBuf,
    pub bash: PathBuf,
    pub sh: PathBuf,
}

/// Everything resolution reads from outside the process, collected up front so
/// the resolution itself is a pure function of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellSearch {
    /// `DEMETEO_BASH_PATH`. When set it is the *only* candidate. An override
    /// that fell through to autodetection when it turned out to be wrong would
    /// hide the user's mistake behind a shell they did not choose, and the
    /// point of the variable is that the user's choice wins.
    pub override_bash: Option<String>,
    /// `InstallPath` under `SOFTWARE\GitForWindows`, HKLM before HKCU, each
    /// hive read under both registry views. The installer writes that key
    /// specifically so third-party tools can find the install; the HKCU copy
    /// is what the non-admin per-user mode leaves behind.
    pub registry_install_paths: Vec<String>,
    /// `git --exec-path` verbatim. It answers with **forward** slashes even on
    /// Windows, which is why [`root_from_exec_path`] pops components with
    /// [`Path::parent`] rather than splitting on a separator.
    pub git_exec_path: Option<String>,
    /// A `git.exe` located on `PATH`. Scoop, Chocolatey and PortableGit
    /// layouts install no registry key, so without this they are invisible.
    pub git_exe: Option<String>,
    pub program_files: Option<String>,
    pub program_files_x86: Option<String>,
    pub local_app_data: Option<String>,
}

/// The two operations resolution cannot perform on strings alone.
pub trait ShellHost {
    fn is_file(&self, path: &Path) -> bool;

    /// Run the candidate as `bash -c 'echo ${BASH_VERSION:-none}'` and return
    /// its stdout. The body is load-bearing: a shell that is not bash leaves
    /// `BASH_VERSION` unset and answers `none`, which is the only signal that
    /// separates the BusyBox MinGit build from a real one.
    fn bash_version(&self, bash: &Path) -> Result<String, String>;
}

/// Why one candidate was passed over. Carried into [`ShellMissing`] so a
/// remediation can say which of the several distinct failures happened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unusable {
    #[error("git-bash.exe is the mintty launcher: it detaches and reports no exit code")]
    MinttyLauncher,
    #[error("this is the WSL launcher, which cannot resolve the Windows paths Demeteo passes")]
    WslLauncher,
    #[error("no such file")]
    Absent,
    #[error("answered {0:?} for BASH_VERSION, so it is not bash")]
    NotBash(String),
    #[error("could not be run: {0}")]
    Unrunnable(String),
}

/// No POSIX shell on this machine, and enough detail to say what to install.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShellMissing {
    #[error("DEMETEO_BASH_PATH points at {}: {reason}", path.display())]
    OverrideUnusable { path: PathBuf, reason: Unusable },
    #[error("{} answered {answer:?} for BASH_VERSION, so it is not bash", bash.display())]
    NotBash { bash: PathBuf, answer: String },
    #[error("Git is installed here but ships no bash.exe")]
    GitWithoutBash {
        git_roots: Vec<PathBuf>,
        searched: Vec<PathBuf>,
    },
    #[error("no Git for Windows installation was found")]
    NoGitForWindows { searched: Vec<PathBuf> },
}

impl ShellMissing {
    /// Whether this machine has a working `git.exe` and still no usable bash.
    ///
    /// That combination is MinGit, and it is the case where a remediation must
    /// name the product: "install Git" is advice the user has demonstrably
    /// already followed, and repeating it reads as Demeteo not having looked.
    pub fn is_mingit(&self) -> bool {
        matches!(self, Self::NotBash { .. } | Self::GitWithoutBash { .. })
    }
}

/// Resolve the shell, or say why there isn't one.
///
/// A candidate that turns out not to be bash does not end the search — the
/// next one is still tried — because a BusyBox MinGit sitting in
/// `%ProgramFiles%` must not mask a real installation the registry points at.
/// "Validate once" is instead a property of the caller: `posix_shell` resolves
/// a single time per process, not once per command.
pub fn resolve(search: &ShellSearch, host: &dyn ShellHost) -> Result<PosixShell, ShellMissing> {
    if let Some(raw) = &search.override_bash {
        return check(shell_from_bash(&win_path(raw)), host)
            .map_err(|(path, reason)| ShellMissing::OverrideUnusable { path, reason });
    }

    let mut searched = Vec::new();
    let mut not_bash = None;
    for candidate in candidates(search) {
        match check(candidate, host) {
            Ok(shell) => return Ok(shell),
            Err((bash, reason)) => {
                if let (Unusable::NotBash(answer), None) = (&reason, &not_bash) {
                    not_bash = Some((bash.clone(), answer.clone()));
                }
                searched.push(bash);
            }
        }
    }

    if let Some((bash, answer)) = not_bash {
        return Err(ShellMissing::NotBash { bash, answer });
    }
    let git_roots = proven_git_roots(search);
    if git_roots.is_empty() {
        Err(ShellMissing::NoGitForWindows { searched })
    } else {
        Err(ShellMissing::GitWithoutBash {
            git_roots,
            searched,
        })
    }
}

fn check(
    mut candidate: PosixShell,
    host: &dyn ShellHost,
) -> Result<PosixShell, (PathBuf, Unusable)> {
    if let Some(reason) = rejection(&candidate.bash) {
        return Err((candidate.bash, reason));
    }
    if !host.is_file(&candidate.bash) {
        return Err((candidate.bash, Unusable::Absent));
    }
    match host.bash_version(&candidate.bash) {
        Ok(answer) if probe_says_bash(&answer) => {
            if !host.is_file(&candidate.sh) {
                candidate.sh = candidate.bash.clone();
            }
            Ok(candidate)
        }
        Ok(answer) => Err((candidate.bash, Unusable::NotBash(answer.trim().to_string()))),
        Err(e) => Err((candidate.bash, Unusable::Unrunnable(e))),
    }
}

/// Every candidate, in the order they are tried, deduplicated by `bash` path.
pub fn candidates(search: &ShellSearch) -> Vec<PosixShell> {
    if let Some(raw) = &search.override_bash {
        return vec![shell_from_bash(&win_path(raw))];
    }
    let mut out: Vec<PosixShell> = Vec::new();
    for root in search_roots(search) {
        for shell in shells_for_root(&root) {
            if !out.iter().any(|seen| seen.bash == shell.bash) {
                out.push(shell);
            }
        }
    }
    out
}

fn search_roots(search: &ShellSearch) -> Vec<PathBuf> {
    let mut roots = proven_git_roots(search);
    for base in [&search.program_files, &search.program_files_x86]
        .into_iter()
        .flatten()
    {
        push_unique(&mut roots, win_path(base).join("Git"));
    }
    if let Some(base) = &search.local_app_data {
        push_unique(&mut roots, win_path(base).join("Programs").join("Git"));
    }
    roots
}

/// Roots something authoritative pointed at — a key an installer wrote, or a
/// `git.exe` that answered — as opposed to the well-known directories, which
/// are guesses. Whether this is empty is what separates "no Git on this
/// machine" from "Git, but the build without bash".
pub fn proven_git_roots(search: &ShellSearch) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for raw in &search.registry_install_paths {
        push_unique(&mut roots, win_path(raw));
    }
    if let Some(raw) = &search.git_exec_path {
        if let Some(root) = root_from_exec_path(raw) {
            push_unique(&mut roots, root);
        }
    }
    if let Some(raw) = &search.git_exe {
        for root in roots_from_git_exe(raw) {
            push_unique(&mut roots, root);
        }
    }
    roots
}

/// `<root>` from `git --exec-path`, which answers e.g.
/// `C:/Program Files/Git/mingw64/libexec/git-core` — three components deep,
/// and with forward slashes, which is the trap: a split on `\` finds nothing
/// and a naive `rfind` of either separator has to be written twice.
pub fn root_from_exec_path(exec_path: &str) -> Option<PathBuf> {
    let path = win_path(exec_path);
    let root = path.parent()?.parent()?.parent()?;
    (!root.as_os_str().is_empty()).then(|| root.to_path_buf())
}

/// `<root>` candidates from a located `git.exe`.
///
/// The installer's own layout is `<root>\cmd\git.exe`, and Scoop's shim points
/// at the same shape one level deeper. A `PATH` reaching straight into the
/// MSYS tree instead yields `<root>\mingw64\bin\git.exe` or
/// `<root>\usr\bin\git.exe`, where the naive one-level pop gives
/// `<root>\mingw64` — a directory that has a `bin` and no bash in it. Both
/// readings are returned, most-specific first.
pub fn roots_from_git_exe(git_exe: &str) -> Vec<PathBuf> {
    let path = win_path(git_exe);
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    if !matches!(lower_name(dir).as_deref(), Some("bin" | "cmd")) {
        return Vec::new();
    }
    let Some(parent) = dir.parent() else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    if matches!(
        lower_name(parent).as_deref(),
        Some("mingw64" | "mingw32" | "usr")
    ) {
        if let Some(above) = parent.parent() {
            push_unique(&mut roots, above.to_path_buf());
        }
    }
    push_unique(&mut roots, parent.to_path_buf());
    roots
}

/// The interpreters one root could offer, in preference order.
///
/// `<root>\bin` first: it is the wrapper that arranges the MSYS `PATH` view,
/// where `<root>\usr\bin` is raw MSYS2 and leaves a script's `PATH` missing
/// the native git. `<root>\git-bash.exe` appears in neither position and never
/// will — see the module header.
pub fn shells_for_root(root: &Path) -> Vec<PosixShell> {
    [root.join("bin"), root.join("usr").join("bin")]
        .into_iter()
        .map(|dir| PosixShell {
            root: root.to_path_buf(),
            bash: dir.join("bash.exe"),
            sh: dir.join("sh.exe"),
        })
        .collect()
}

/// Read a user-supplied `bash.exe` back into a full [`PosixShell`], deriving
/// the root by undoing whichever of the two layouts it sits in.
pub fn shell_from_bash(bash: &Path) -> PosixShell {
    let dir = bash.parent().unwrap_or(Path::new(""));
    let mut root = dir;
    if lower_name(dir).as_deref() == Some("bin") {
        if let Some(parent) = dir.parent() {
            root = if lower_name(parent).as_deref() == Some("usr") {
                parent.parent().unwrap_or(parent)
            } else {
                parent
            };
        }
    }
    PosixShell {
        root: root.to_path_buf(),
        bash: bash.to_path_buf(),
        sh: dir.join("sh.exe"),
    }
}

/// Whether this path is one of the two impostors, checked on the path alone so
/// the verdict costs no I/O and holds for a candidate that does not exist yet.
///
/// The WSL test is positional, not by name: the launcher *is* called
/// `bash.exe`, and the only thing distinguishing it is that it lives in the
/// Windows system directory. `Sysnative` and `SysWOW64` are the same file seen
/// through the other two redirections.
pub fn rejection(bash: &Path) -> Option<Unusable> {
    if lower_name(bash).as_deref() == Some("git-bash.exe") {
        return Some(Unusable::MinttyLauncher);
    }
    let in_system_dir = bash.components().any(|c| {
        matches!(
            c.as_os_str()
                .to_string_lossy()
                .to_ascii_lowercase()
                .as_str(),
            "system32" | "sysnative" | "syswow64"
        )
    });
    in_system_dir.then_some(Unusable::WslLauncher)
}

/// The verdict on `echo ${BASH_VERSION:-none}`. Empty output counts as a
/// refusal too: a shell that printed nothing did not evaluate the expansion.
pub fn probe_says_bash(stdout: &str) -> bool {
    let answer = stdout.trim();
    !answer.is_empty() && answer != "none"
}

/// Parse a Windows path from any of the three sources, all of which spell it
/// differently. Backslashes become forward slashes because Windows accepts
/// either everywhere and `\` is not a legal filename character, so the rewrite
/// is lossless — and it leaves one [`Path`] implementation that behaves the
/// same on the Linux host these functions are tested on.
fn win_path(raw: &str) -> PathBuf {
    PathBuf::from(raw.trim().trim_matches('"').replace('\\', "/"))
}

fn lower_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
}

fn push_unique(out: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() || out.contains(&path) {
        return;
    }
    out.push(path);
}

/// The resolved shell for this process, or the failure that explains itself.
///
/// Resolution runs at most once: the registry reads and the probe spawn are
/// the expensive part, and a machine does not grow a Git installation between
/// two commands often enough to pay for them per invocation.
#[cfg(windows)]
pub fn posix_shell() -> Result<&'static PosixShell, ShellMissing> {
    use std::sync::OnceLock;

    static RESOLVED: OnceLock<Result<PosixShell, ShellMissing>> = OnceLock::new();
    RESOLVED
        .get_or_init(|| resolve(&super::discovery::search(), &super::discovery::WindowsHost))
        .as_ref()
        .map_err(Clone::clone)
}

#[cfg(test)]
#[path = "../../../tests/shared/win/posix_shell.rs"]
mod tests;
