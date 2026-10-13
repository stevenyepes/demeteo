//! What a force push of a run's branch may overwrite on `origin`. See
//! [`crate::domain`].
//!
//! The two pushes that rewrite a branch — the runner's terminal push and the
//! merge-request publisher's — exist to re-point a branch *this clone* pushed
//! before: a retried or replayed run, or a `finalize` squash under an open MR.
//! A plain `-f` also overwrites whatever anybody else published there since,
//! and the case that made it matter is a sync resolution published from the
//! desktop's clone while the runner's clone still held the pre-sync branch: the
//! runner's next terminal push silently removed the resolution from the PR.
//!
//! So both lease against the clone's own `refs/remotes/origin/<branch>`, which
//! a successful push updates — it is exactly "what this clone last pushed or
//! fetched". Every legitimate re-push agrees with it, history rewrites
//! included, because the lease compares origin's tip and not ancestry.
//!
//! The expected value is always spelled out. The bare `--force-with-lease`
//! reads the same tracking ref but silently degrades to a blind force when the
//! ref is absent; here an absent ref is the explicit empty expectation, which
//! git reads as "the branch must not exist on origin yet".
//!
//! **Never fetch the branch before leasing against it.** A fetch moves the
//! tracking ref to whatever origin holds, turning the lease back into `-f`.

/// The expectation one push leases against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushLease {
    pub branch: String,
    /// Origin's tip as this clone last saw it; `None` means the branch must
    /// not exist on origin.
    pub expected: Option<String>,
}

/// The remote-tracking ref the lease is read from.
pub fn tracking_ref(branch: &str) -> String {
    format!("refs/remotes/origin/{branch}")
}

/// The arguments of a `git for-each-ref` reading one ref's tip, which
/// [`ref_tip`] parses. `for-each-ref` rather than `rev-parse` because an absent
/// ref is a successful empty answer, not an error that would have to be told
/// apart from a git that failed for another reason.
pub fn ref_query(refname: &str) -> [String; 3] {
    [
        "for-each-ref".to_string(),
        "--format=%(refname) %(objectname)".to_string(),
        refname.to_string(),
    ]
}

/// [`ref_query`] for the lease's tracking ref.
pub fn tracking_query(branch: &str) -> [String; 3] {
    ref_query(&tracking_ref(branch))
}

/// `refname`'s tip in [`ref_query`]'s output, matched on the exact name:
/// `for-each-ref` treats its pattern as a path prefix, so `origin/a` would also
/// list an `origin/a/b`.
pub fn ref_tip(for_each_ref_output: &str, refname: &str) -> Option<String> {
    for_each_ref_output.lines().find_map(|line| {
        let (name, sha) = line.trim().split_once(' ')?;
        (name == refname && !sha.is_empty()).then(|| sha.to_string())
    })
}

/// The lease for `branch`, from [`tracking_query`]'s output.
pub fn lease_from_tracking(branch: &str, for_each_ref_output: &str) -> PushLease {
    PushLease {
        branch: branch.to_string(),
        expected: ref_tip(for_each_ref_output, &tracking_ref(branch)),
    }
}

impl PushLease {
    /// The flag that replaces `-f`.
    pub fn flag(&self) -> String {
        format!(
            "--force-with-lease=refs/heads/{}:{}",
            self.branch,
            self.expected.as_deref().unwrap_or("")
        )
    }

    /// What a push this lease refused tells the user. Non-retryable by
    /// construction: pushing again from the same clone fails the same way
    /// until something brings the clone up to date with origin.
    pub fn refusal(&self) -> String {
        let expected = match self.expected.as_deref() {
            Some(sha) => format!("at {sha}"),
            None => "absent".to_string(),
        };
        format!(
            "origin/{branch} has moved since this clone last pushed it (expected it {expected}), \
             so the push was refused rather than overwrite it. The usual cause is a sync \
             resolution published from the desktop; bring the runner's copy of the branch up to \
             date (sync again from the desktop) and retry.",
            branch = self.branch
        )
    }
}

/// Whether a failed push was refused by its lease — git's `stale info`
/// rejection — as opposed to any other failure.
pub fn is_stale_lease(error: &str) -> bool {
    error
        .lines()
        .any(|line| line.contains("! [rejected]") && line.contains("(stale info)"))
}

#[cfg(test)]
#[path = "../../tests/domain/push_lease.rs"]
mod tests;
