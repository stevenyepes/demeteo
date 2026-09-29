//! Whether a machine has room for a new per-feature dependency cache.
//! See [`crate::domain`].
//!
//! A cache ([`feature_cache_dir`](crate::paths::feature_cache_dir)) is seeded
//! from the clone's `node_modules` and `target/` — 12 to 56 GB — and the build
//! that follows writes into it. Out of room, that build dies halfway on an
//! `ENOSPC` that reads like any other red harness, and the retry loop hands it
//! to an agent that cannot free a byte. So the question is asked once, before
//! the seed, and a "no" ends the run naming the disk rather than the code.

/// Free space below which a new cache is not seeded: the low end of what one
/// feature's cache and its first build occupy, because the seed's own cost is
/// unknowable without walking the tree (a reflink copy is near-free, a plain
/// copy is the whole size) and the build writes into it either way.
pub const SEED_FLOOR_BYTES: u64 = 20 * GIB;

const GIB: u64 = 1024 * 1024 * 1024;

/// Leads the `Err` a refused seed provisions with, and is how the step
/// executor tells it from a provisioning hiccup worth one more try:
/// [`is_seed_refusal`] reads it anywhere in a message, since each caller wraps
/// the error in its own context before it reaches the outcome.
pub const DISK_FULL_ERROR_PREFIX: &str = "disk full: ";

pub fn is_seed_refusal(error: &str) -> bool {
    error.contains(DISK_FULL_ERROR_PREFIX)
}

/// Where the probe goes, given the nearest path of the cache's ancestry that
/// exists on the machine.
#[derive(Debug, PartialEq, Eq)]
pub enum SeedProbe<'a> {
    /// The cache exists: a later step of the same feature reuses it, and the
    /// disk it already occupies is not a reason to stop that feature.
    AlreadySeeded,
    Probe(&'a str),
    /// Nothing on the path exists or could be read.
    Unknown,
}

pub fn seed_probe<'a>(cache_dir: &str, nearest_existing: Option<&'a str>) -> SeedProbe<'a> {
    match nearest_existing {
        Some(existing) if existing == cache_dir => SeedProbe::AlreadySeeded,
        Some(existing) => SeedProbe::Probe(existing),
        None => SeedProbe::Unknown,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SeedSpace {
    Proceed,
    ReclaimThenRecheck,
    Refuse(String),
}

/// What the refusal names, so the reader knows which disk to clear.
pub struct SeedSite<'a> {
    pub machine: &'a str,
    pub cache_dir: &'a str,
    /// The existing ancestor the free space was read at.
    pub probed: &'a str,
}

/// `free` is `None` when the machine could not answer; that proceeds, as every
/// seed did before this check, because refusing on a probe that never ran would
/// stop a feature on a machine with room to spare. `reclaim_spent` is also true
/// when there is nothing to reclaim with.
pub fn seed_space(site: &SeedSite<'_>, free: Option<u64>, reclaim_spent: bool) -> SeedSpace {
    match free {
        None => SeedSpace::Proceed,
        Some(free) if free >= SEED_FLOOR_BYTES => SeedSpace::Proceed,
        Some(_) if !reclaim_spent => SeedSpace::ReclaimThenRecheck,
        Some(free) => SeedSpace::Refuse(format!(
            "{DISK_FULL_ERROR_PREFIX}machine '{}' has {} free at {}, below the {} a new \
             dependency cache at {} needs, after reclaiming leaked caches. Free space on \
             that disk, then retry the run.",
            site.machine,
            gib(free),
            site.probed,
            gib(SEED_FLOOR_BYTES),
            site.cache_dir,
        )),
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / GIB as f64)
}

/// `path`, then each of its parents up to the root, nearest first.
///
/// Both `/` and `\` separate, whatever the host: the target may be a Windows
/// desktop's own disk or a Linux remote, and this runs on either.
pub fn ancestor_candidates(path: &str) -> Vec<String> {
    let is_sep = |c: char| c == '/' || c == '\\';
    let mut out = Vec::new();
    let mut current = path.trim_end_matches(is_sep);
    if current.is_empty() {
        if path.starts_with(is_sep) {
            out.push(path[..1].to_string());
        }
        return out;
    }
    loop {
        if is_drive(current) {
            let rooted = &path[..(current.len() + 1).min(path.len())];
            out.push(rooted.to_string());
            return out;
        }
        out.push(current.to_string());
        let Some(cut) = current.rfind(is_sep) else {
            return out;
        };
        let parent = current[..cut].trim_end_matches(is_sep);
        if parent.is_empty() {
            out.push(current[..1].to_string());
            return out;
        }
        current = parent;
    }
}

fn is_drive(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Bytes available to an unprivileged writer, from `df -Pk <path>`.
///
/// The record is found by shape rather than by column: three counts and a
/// `N%` capacity, the available count being the last of the three. Column
/// positions do not survive a filesystem name or mount point with a space in
/// it, and a `df` that ignored `-P` wraps a long name onto a line of its own.
pub fn parse_df_available(output: &str) -> Result<u64, String> {
    let tokens: Vec<&str> = output.split_whitespace().collect();
    let is_count = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    let is_capacity = |t: &str| t.strip_suffix('%').is_some_and(is_count);
    let available = tokens
        .windows(4)
        .find(|w| is_count(w[0]) && is_count(w[1]) && is_count(w[2]) && is_capacity(w[3]))
        .map(|w| w[2])
        .ok_or_else(|| format!("df answered no usable record: {}", output.trim()))?;
    available
        .parse::<u64>()
        .ok()
        .and_then(|kib| kib.checked_mul(1024))
        .ok_or_else(|| format!("df reported an out-of-range count: {available}"))
}

#[cfg(test)]
#[path = "../../tests/domain/seed_space.rs"]
mod tests;
