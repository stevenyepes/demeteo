//! Whether a machine's `demeteo-runner` is this app's build. See
//! [`crate::domain`]; this module is the one home of the policy, and other
//! code cites it rather than restating it.
//!
//! **Compatible means equal.** A runner is compatible only when its build
//! version is structurally the app's build version. Any other pair is a
//! mismatch with a direction — the runner is behind (upgrade the runner) or
//! ahead (upgrade Demeteo) — because the runner and the app speak an RPC
//! surface that has no negotiation, so "close enough" is not a state either
//! side can check.
//!
//! **The channel is read from the version string alone.** A `-N` suffix is a
//! nightly build, none is stable. Nothing else — not the app's compiled-in
//! release channel, not a debug build — decides it, because the runner has no
//! other way to say which channel it came from.
//!
//! **What cannot be read is never a mismatch.** An unreachable machine or an
//! unparseable reading is [`RunnerCompatibility::Unknown`], and a binary that
//! printed nothing is [`RunnerCompatibility::NotInstalled`]; neither is
//! reported as older or newer.

use std::cmp::Ordering;
use std::fmt;

use serde::Serialize;

const MAX_READING_LEN: usize = 64;
const RUNNER_PREFIX: &str = "demeteo-runner ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    Stable,
    Nightly,
}

impl fmt::Display for ReleaseChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        })
    }
}

/// A Demeteo release build: stable `X.Y.Z`, or nightly `X.Y.Z-N` with N ≥ 1.
///
/// Nightly `X.Y.Z-N` is built after stable `X.Y.Z`; do not use SemVer
/// precedence, which ranks it before. The derived order is
/// `(major, minor, patch, build)` with `build` 0 for stable — field order is
/// load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReleaseVersion {
    major: u32,
    minor: u32,
    patch: u32,
    build: u32,
}

impl ReleaseVersion {
    /// Parses a bare version or a `demeteo-runner --version` line, tolerating
    /// the surrounding whitespace an SSH capture carries. Anything that is not
    /// exactly a release build is `None`: a pre-release tag, a `-0` build, a
    /// leading zero, or a reading longer than 64 bytes.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.len() > MAX_READING_LEN {
            return None;
        }
        let raw = raw.strip_prefix(RUNNER_PREFIX).map_or(raw, str::trim_start);
        let (base, build) = match raw.split_once('-') {
            Some((base, build)) => (base, number(build).filter(|&n| n >= 1)?),
            None => (raw, 0),
        };
        let mut parts = base.split('.');
        let version = Self {
            major: number(parts.next()?)?,
            minor: number(parts.next()?)?,
            patch: number(parts.next()?)?,
            build,
        };
        parts.next().is_none().then_some(version)
    }

    pub fn channel(&self) -> ReleaseChannel {
        if self.build == 0 {
            ReleaseChannel::Stable
        } else {
            ReleaseChannel::Nightly
        }
    }
}

/// A canonical decimal: digits only, no sign, no leading zero.
fn number(part: &str) -> Option<u32> {
    let canonical = !part.is_empty()
        && part.bytes().all(|b| b.is_ascii_digit())
        && (part == "0" || !part.starts_with('0'));
    canonical.then(|| part.parse().ok()).flatten()
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if self.build > 0 {
            write!(f, "-{}", self.build)?;
        }
        Ok(())
    }
}

/// What the probe learned, before policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerVersionReading {
    /// The raw string from `health.build_version` or `--version`.
    Reported(String),
    /// `--version` produced no output.
    NotInstalled,
    /// Why no reading could be taken.
    Unreachable(String),
}

/// Version strings are the canonical [`ReleaseVersion`] rendering, except
/// `app` in `Unknown`/`NotInstalled`, which is the app's own string as given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum RunnerCompatibility {
    Compatible {
        version: String,
        channel: ReleaseChannel,
    },
    RunnerBehind {
        runner: String,
        runner_channel: ReleaseChannel,
        app: String,
        app_channel: ReleaseChannel,
    },
    RunnerAhead {
        runner: String,
        runner_channel: ReleaseChannel,
        app: String,
        app_channel: ReleaseChannel,
    },
    NotInstalled {
        app: String,
        app_channel: ReleaseChannel,
    },
    /// `app_channel` is `Stable` when the app's own version did not parse.
    Unknown {
        app: String,
        app_channel: ReleaseChannel,
        detail: String,
    },
}

impl RunnerCompatibility {
    pub fn is_compatible(&self) -> bool {
        matches!(self, Self::Compatible { .. })
    }

    /// The user-facing sentence for this verdict on `machine`.
    pub fn message(&self, machine: &str) -> String {
        match self {
            Self::Compatible { version, channel } => {
                format!("demeteo-runner {version} ({channel}) on {machine} matches Demeteo.")
            }
            Self::RunnerBehind {
                runner,
                runner_channel,
                app,
                app_channel,
            } => format!(
                "demeteo-runner {runner} ({runner_channel}) on {machine} is older than Demeteo \
                 {app} ({app_channel}) — upgrade the runner from Machines settings.{}",
                channel_suffix(*runner_channel, *app_channel)
            ),
            Self::RunnerAhead {
                runner,
                runner_channel,
                app,
                app_channel,
            } => format!(
                "demeteo-runner {runner} ({runner_channel}) on {machine} is newer than Demeteo \
                 {app} ({app_channel}) — upgrade Demeteo to match, or push this app's runner \
                 from Machines settings.{}",
                channel_suffix(*runner_channel, *app_channel)
            ),
            Self::NotInstalled { .. } => format!(
                "demeteo-runner is not installed on {machine} — enable remote runs from Machines \
                 settings."
            ),
            Self::Unknown { detail, .. } => format!(
                "Couldn't verify the demeteo-runner version on {machine} ({detail}) — check the \
                 machine in Machines settings."
            ),
        }
    }
}

fn channel_suffix(runner: ReleaseChannel, app: ReleaseChannel) -> String {
    if runner == app {
        return String::new();
    }
    format!(
        " The runner is on the {runner} channel and Demeteo on {app}; both must be on the same \
         channel and version."
    )
}

/// The verdict for a runner that gave `runner`, against the app's own `app`
/// version.
pub fn assess(app: &str, runner: RunnerVersionReading) -> RunnerCompatibility {
    let Some(app_version) = ReleaseVersion::parse(app) else {
        let app = app.trim().to_string();
        return RunnerCompatibility::Unknown {
            detail: format!("app version {app} is not a release version"),
            app,
            app_channel: ReleaseChannel::Stable,
        };
    };
    let app = app_version.to_string();
    let app_channel = app_version.channel();
    let raw = match runner {
        RunnerVersionReading::Reported(raw) => raw,
        RunnerVersionReading::NotInstalled => {
            return RunnerCompatibility::NotInstalled { app, app_channel }
        }
        RunnerVersionReading::Unreachable(detail) => {
            return RunnerCompatibility::Unknown {
                app,
                app_channel,
                detail,
            }
        }
    };
    let Some(runner_version) = ReleaseVersion::parse(&raw) else {
        let shown: String = raw.trim().chars().take(MAX_READING_LEN).collect();
        return RunnerCompatibility::Unknown {
            app,
            app_channel,
            detail: format!("runner reported {shown:?}, which is not a release version"),
        };
    };
    let runner = runner_version.to_string();
    let runner_channel = runner_version.channel();
    match runner_version.cmp(&app_version) {
        Ordering::Equal => RunnerCompatibility::Compatible {
            version: app,
            channel: app_channel,
        },
        Ordering::Less => RunnerCompatibility::RunnerBehind {
            runner,
            runner_channel,
            app,
            app_channel,
        },
        Ordering::Greater => RunnerCompatibility::RunnerAhead {
            runner,
            runner_channel,
            app,
            app_channel,
        },
    }
}

#[cfg(test)]
#[path = "../../tests/domain/runner_version.rs"]
mod tests;
