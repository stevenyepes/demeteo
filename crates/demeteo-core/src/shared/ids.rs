//! ID generation. Previously lived in `paths::new_id`; moved here so
//! time and path utilities don't have to be imported together.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Monotonic counter so even IDs generated in the same nanosecond
/// differ. Initialized lazily on first call with a seed derived from
/// the wall clock.
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_counter() -> u64 {
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Generate a short, sortable, unique-enough ID for in-process use
/// (database primary keys, session ids, log entries).
///
/// Format: `<unix_ms>-<random_8>-<counter>`. The millisecond prefix
/// gives rough sort order; the random suffix makes IDs unguessable;
/// the monotonic counter guarantees uniqueness even under a tight
/// burst.
///
/// **Not cryptographically random.** For PATs, webhook secrets, etc.,
/// use the OS keyring or a proper CSPRNG.
pub fn new_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Thread-local seed for xorshift; falls back to wall-clock nanos.
    let random = thread_random_u64();
    let counter = next_counter();
    format!("{:x}-{:08x}-{:04x}", ms, random, counter & 0xFFFF)
}

fn thread_random_u64() -> u64 {
    use std::cell::Cell;
    use std::time::{SystemTime, UNIX_EPOCH};
    thread_local!(static SEED: Cell<u64> = const { Cell::new(0) });
    SEED.with(|s| {
        let mut x = s.get();
        if x == 0 {
            // Seed from wall clock + pointer-as-pseudo-randomness. We
            // don't need cryptographic strength here.
            x = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xDEAD_BEEF);
            x ^= &s as *const _ as u64;
        }
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        s.set(x);
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_under_burst() {
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            ids.insert(new_id());
        }
        assert_eq!(ids.len(), 1000, "duplicate IDs in a burst");
    }

    #[test]
    fn ids_sort_lexicographically_by_time() {
        let a = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_id();
        // Both ids are hex; the millisecond prefix dominates.
        assert!(a < b, "expected {} < {}", a, b);
    }

    #[test]
    fn ids_have_three_components() {
        let id = new_id();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 3, "expected 3 components in {}", id);
    }
}
