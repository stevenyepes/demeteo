//! Time helpers. Previously in `paths::now_ms` / `paths::now_secs`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current Unix epoch time in milliseconds. Returns 0 if the system
/// clock is before the epoch (effectively impossible on a real OS).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Current Unix epoch time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "../../tests/shared/time.rs"]
mod tests;
