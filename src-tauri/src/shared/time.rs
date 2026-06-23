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
mod tests {
    use super::*;

    #[test]
    fn now_is_monotonic_within_a_test() {
        let a = now_ms();
        let b = now_ms();
        assert!(b >= a);
    }

    #[test]
    fn seconds_less_than_milliseconds() {
        let ms = now_ms();
        let s = now_secs();
        // ms should be roughly 1000x s, give or take a few ms.
        assert!(ms / 1000 <= s + 1);
        assert!(s * 1000 <= ms + 1000);
    }
}
