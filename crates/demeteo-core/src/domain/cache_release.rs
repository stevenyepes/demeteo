//! When a feature's dependency cache may be deleted. See [`crate::domain`].
//!
//! The cache ([`feature_cache_dir`](crate::paths::feature_cache_dir)) holds a
//! whole `node_modules` / `target` — tens of gigabytes — and is pure derived
//! state: provisioning reseeds it from the primary checkout whenever it is
//! missing. So the only question is whether the feature can still *run*, and
//! that is not what the project's `feature_lifecycle` policy answers. `keep` and
//! `archive` decide the fate of the branch and the row; tying the cache to them
//! is how every merged feature under `keep` leaked one, forever.
//!
//! Conservative on purpose. Replay refuses only a `running` feature, so
//! `failed`, `cancelled`, `awaiting_mr` and a `completed` feature whose PR is
//! still open are all one click from running again, and deleting their cache
//! would turn that click into a cold install. A merged or closed PR, or a
//! feature the user archived or deleted, is where work on it has ended.
//!
//! Until nobody has touched it for the project's `cache_idle_ttl_days`
//! ([`cache_releasable_at`]). A feature left `failed` for a month is still one
//! click from running, but holding tens of gigabytes against that click is the
//! worse trade: the click costs one cold install, the hold costs the disk.
//! Only statuses no driver owns age out — a live run never does, however long
//! it has been parked, because deleting under it breaks the run rather than
//! slowing the next one.

/// Whether a feature in `status`, with its PR in `mr_state`, has a dependency
/// cache that nothing will read again.
pub fn cache_releasable(status: &str, mr_state: Option<&str>) -> bool {
    match status {
        "deleted" | "archived" => true,
        "completed" => matches!(mr_state, Some("merged" | "closed")),
        _ => false,
    }
}

/// How long an idle cache is kept when the project has not said.
pub const DEFAULT_CACHE_IDLE_TTL_DAYS: u32 = 14;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The project's `cache_idle_ttl_days` as a sweep applies it: unset is
/// [`DEFAULT_CACHE_IDLE_TTL_DAYS`], and `0` switches idle release off.
pub fn cache_idle_ttl_days(setting: Option<u32>) -> u32 {
    setting.unwrap_or(DEFAULT_CACHE_IDLE_TTL_DAYS)
}

/// Whether activity last seen at `last_activity_ms` is more than `ttl_days`
/// before `now_ms`. Never, when `ttl_days` is `0`.
pub fn idle_past_ttl(last_activity_ms: i64, now_ms: i64, ttl_days: u32) -> bool {
    ttl_days != 0 && now_ms.saturating_sub(last_activity_ms) > i64::from(ttl_days) * DAY_MS
}

/// [`cache_releasable`], or a feature no driver owns that has sat idle past
/// the project's TTL.
///
/// An allowlist of idle statuses rather than a list of live ones: a status
/// added later is kept until someone decides it is idle, where a denylist
/// would age out a live run it had never heard of. `completed` is here for a
/// PR still open, which is as abandoned as `awaiting_mr` after two weeks.
pub fn cache_releasable_at(
    status: &str,
    mr_state: Option<&str>,
    last_activity_ms: i64,
    now_ms: i64,
    ttl_days: u32,
) -> bool {
    cache_releasable(status, mr_state)
        || (matches!(
            status,
            "failed" | "cancelled" | "awaiting_mr" | "interrupted" | "completed"
        ) && idle_past_ttl(last_activity_ms, now_ms, ttl_days))
}

/// [`cache_releasable`] for a feature in `status` whose PR the MR monitor has
/// just recorded as `new_mr_state`. The monitor writes a merge as `completed`
/// whatever the status was, and leaves the status of every other transition
/// alone.
pub fn releasable_after_mr_poll(status: &str, new_mr_state: &str) -> bool {
    let status = if new_mr_state == "merged" {
        "completed"
    } else {
        status
    };
    cache_releasable(status, Some(new_mr_state))
}

#[cfg(test)]
#[path = "../../tests/domain/cache_release.rs"]
mod tests;
