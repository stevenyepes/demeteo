use serde::{Deserialize, Serialize};

/// The operating system a command or an agent process actually lands on.
///
/// A property of the *machine*, never of the build. The desktop ships on three
/// hosts and any of them can drive a Linux remote, so `cfg!(windows)` answers a
/// question about the running binary and not about the target — which is why
/// this is resolved through `ExecutionPort::resolve_platform` rather than read
/// from a constant. Everything a POSIX assumption used to ride on (a `sh` that
/// exists, `/`-rooted paths, a `$SHELL` worth forwarding) is a property of this
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Linux,
    MacOS,
    Windows,
}

impl Platform {
    /// Every platform Demeteo ships a desktop for — mirrors
    /// [`EffortLevel::ALL`](crate::domain::models::EffortLevel::ALL).
    pub const ALL: [Platform; 3] = [Platform::Linux, Platform::MacOS, Platform::Windows];

    /// Whether the target speaks POSIX: a `sh` on `PATH`, `/`-rooted absolute
    /// paths, `$SHELL`/`$TMPDIR`/`$HOME` meaning what a shell script expects.
    ///
    /// Two of three variants, so the negation is the interesting one — this
    /// exists to be asked rather than to be `match`ed, because `!= Windows`
    /// spreads a third platform's answer across every call site the day one is
    /// added.
    pub const fn is_posix(self) -> bool {
        matches!(self, Platform::Linux | Platform::MacOS)
    }

    /// The stable lowercase identifier used on the wire and in prompts. It is
    /// deliberately Rust's `std::env::consts::OS` spelling, so a value that has
    /// been through [`from_target_os`](Self::from_target_os) round-trips.
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::MacOS => "macos",
            Platform::Windows => "windows",
        }
    }

    /// Interpret a `std::env::consts::OS` value.
    ///
    /// `None` for anything Demeteo does not ship a desktop for (a BSD, an
    /// illumos) rather than a nearest-neighbour guess: the caller is a
    /// transport answering "what is this host", and a wrong answer there is
    /// indistinguishable from a right one downstream.
    pub fn from_target_os(os: &str) -> Option<Self> {
        match os {
            "linux" => Some(Platform::Linux),
            "macos" => Some(Platform::MacOS),
            "windows" => Some(Platform::Windows),
            _ => None,
        }
    }

    /// Interpret the `sysname` a `uname -s` prints.
    ///
    /// Only the two POSIX kernels are named. A Windows box has no `uname` in
    /// the first place, and the MSYS/Cygwin spellings that would answer one
    /// (`MINGW64_NT-10.0`, `CYGWIN_NT-10.0`) are rejected on purpose: remote
    /// execution is Linux-only (R2, `docs/REMOTE_EXECUTION.md`), so a probe
    /// that came back in one of those forms means the target is not what the
    /// caller believes, and mapping it to [`Platform::Windows`] would hand a
    /// native-Windows answer to a transport that only ever speaks POSIX to it.
    pub fn from_uname(sysname: &str) -> Option<Self> {
        match sysname.trim() {
            "Linux" => Some(Platform::Linux),
            "Darwin" => Some(Platform::MacOS),
            _ => None,
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/models/platform.rs"]
mod tests;
