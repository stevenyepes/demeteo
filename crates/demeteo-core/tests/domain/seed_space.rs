// Tests for `src/domain/seed_space.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::*;

const SITE: SeedSite<'static> = SeedSite {
    machine: "m-build",
    cache_dir: "/srv/repo_cache_feature-x",
    probed: "/srv",
};

#[test]
fn room_to_spare_proceeds_without_reclaiming() {
    assert_eq!(
        seed_space(&SITE, Some(SEED_FLOOR_BYTES), false),
        SeedSpace::Proceed
    );
    assert_eq!(seed_space(&SITE, Some(u64::MAX), true), SeedSpace::Proceed);
}

#[test]
fn short_of_room_reclaims_once_before_refusing() {
    assert_eq!(
        seed_space(&SITE, Some(SEED_FLOOR_BYTES - 1), false),
        SeedSpace::ReclaimThenRecheck
    );
    assert!(matches!(
        seed_space(&SITE, Some(SEED_FLOOR_BYTES - 1), true),
        SeedSpace::Refuse(_)
    ));
}

/// A probe that never ran is not evidence of a full disk.
#[test]
fn an_unanswered_probe_proceeds() {
    assert_eq!(seed_space(&SITE, None, false), SeedSpace::Proceed);
    assert_eq!(seed_space(&SITE, None, true), SeedSpace::Proceed);
}

#[test]
fn a_refusal_names_the_machine_the_path_and_both_sizes() {
    let SeedSpace::Refuse(message) = seed_space(&SITE, Some(3 * GIB / 2), true) else {
        panic!("a spent reclaim on a short disk refuses");
    };
    assert!(message.starts_with(DISK_FULL_ERROR_PREFIX), "{message}");
    for needle in [
        "m-build",
        "/srv/repo_cache_feature-x",
        "at /srv",
        "1.5 GiB",
        "20.0 GiB",
    ] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from {message}"
        );
    }
    assert!(is_seed_refusal(&format!(
        "sequence step: worktree provision failed (f-1): {message}"
    )));
}

#[test]
fn only_a_refusal_reads_as_one() {
    assert!(!is_seed_refusal(
        "git worktree add failed: No space left on device"
    ));
    assert!(!is_seed_refusal("transport: connection reset"));
}

#[test]
fn an_existing_cache_is_reused_without_a_probe() {
    let cache = "/srv/repo_cache_f";
    assert_eq!(seed_probe(cache, Some(cache)), SeedProbe::AlreadySeeded);
    assert_eq!(seed_probe(cache, Some("/srv")), SeedProbe::Probe("/srv"));
    assert_eq!(seed_probe(cache, None), SeedProbe::Unknown);
}

#[test]
fn ancestors_run_nearest_first_to_the_posix_root() {
    assert_eq!(
        ancestor_candidates("/srv/work/repo_cache_f/"),
        ["/srv/work/repo_cache_f", "/srv/work", "/srv", "/"]
    );
    assert_eq!(ancestor_candidates("/"), ["/"]);
    assert_eq!(ancestor_candidates("a//b"), ["a//b", "a"]);
    assert!(ancestor_candidates("").is_empty());
}

/// Spelled with backslashes on purpose, and asserted on every host: the walk
/// splits on `\` itself rather than through `std::path`, which on Linux would
/// read each of these as one filename and pass by answering nothing.
#[test]
fn ancestors_stop_at_a_windows_drive_root() {
    assert_eq!(
        ancestor_candidates(r"C:\Users\dev\repo_cache_f"),
        [
            r"C:\Users\dev\repo_cache_f",
            r"C:\Users\dev",
            r"C:\Users",
            r"C:\"
        ]
    );
    assert_eq!(
        ancestor_candidates("D:/work/x"),
        ["D:/work/x", "D:/work", "D:/"]
    );
    assert_eq!(ancestor_candidates(r"C:\"), [r"C:\"]);
}

#[test]
fn df_available_is_read_in_bytes() {
    let out = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
               /dev/nvme0n1p2   975583240 612345678 314567890      67% /\n";
    assert_eq!(parse_df_available(out), Ok(314_567_890 * 1024));
}

#[test]
fn df_a_full_disk_reads_as_zero() {
    let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
               /dev/sda1 1000 1000 0 100% /\n";
    assert_eq!(parse_df_available(out), Ok(0));
}

#[test]
fn df_names_with_spaces_do_not_shift_the_columns() {
    let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
               //nas/team share 1000 400 600 40% /mnt/team share\n";
    assert_eq!(parse_df_available(out), Ok(600 * 1024));
    let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
               /dev/sdb1 1000 300 700 30% /mnt/100% full\n";
    assert_eq!(parse_df_available(out), Ok(700 * 1024));
}

/// What `-P` exists to prevent, read correctly anyway.
#[test]
fn df_a_wrapped_long_filesystem_name_is_read_across_the_break() {
    let out = "Filesystem           1K-blocks      Used Available Use% Mounted on\n\
               /dev/mapper/vg_build_host_with_a_long_name-lv_root\n\
               \x20                    51475068  41234567   7609076  85% /\n";
    assert_eq!(parse_df_available(out), Ok(7_609_076 * 1024));
}

#[test]
fn df_without_a_usable_record_is_an_error() {
    assert!(parse_df_available("").is_err());
    assert!(
        parse_df_available("Filesystem 1024-blocks Used Available Capacity Mounted on\n").is_err()
    );
    assert!(parse_df_available("proc - - - - /proc\n").is_err());
    assert!(parse_df_available("df: /nope: No such file or directory\n").is_err());
}

#[test]
fn df_an_overflowing_count_is_an_error_not_a_wrap() {
    let out = format!("fs 1 1 {} 1% /\n", u64::MAX);
    assert!(parse_df_available(&out).is_err());
}
