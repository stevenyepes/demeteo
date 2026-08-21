// Tests for `src/domain/dependency_cache.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// These paths come from the repository, not from this crate, so the interesting
// cases are all adversarial. Each rejection below corresponds to a thing a
// directory in someone's checkout may legally be called.

use super::*;

/// Joined with NUL because `-z` is, and because the separator is the only thing
/// telling two paths apart once one of them contains a newline.
fn probe(entries: &[&str]) -> String {
    entries.join("\0")
}

/// The bug this module exists for: git reports the cache where the project
/// actually keeps it, and every one of these was invisible to a probe that only
/// looked at `<repo>/<name>`.
#[test]
fn a_cache_is_found_wherever_the_project_keeps_it() {
    assert_eq!(
        shareable_cache_paths(&probe(&[
            "node_modules/",
            "src-tauri/target/",
            "packages/web/node_modules/",
            "apps/api/.next/",
            "services/go/vendor/",
            "deep/a/b/target/",
        ])),
        vec![
            "node_modules",
            "src-tauri/target",
            "packages/web/node_modules",
            "apps/api/.next",
            "services/go/vendor",
            "deep/a/b/target",
        ]
    );
}

/// git also reports ignored directories that are not caches, and — when
/// everything beneath them is ignored — the *parent* directories on the way
/// down. Only the last segment is matched, so a parent named `apps` names
/// nothing even though its child does.
#[test]
fn only_the_last_segment_decides_and_everything_else_git_says_is_dropped() {
    assert_eq!(
        shareable_cache_paths(&probe(&[
            "dist/",
            "src-tauri/gen/",
            "src-tauri/gen/schemas/",
            "apps/",
            "deep/a/",
            "target-notes/",
            "node_modules.bak/",
            // A known name in a *non-final* segment is the trap: this is a
            // directory inside a cache, not a cache. Linking it would seed and
            // symlink at the wrong granularity and write an exclusion for a
            // path the real cache still shadows.
            "target/debug/",
            "vendor/logs/",
            "node_modules/.cache/",
        ])),
        Vec::<String>::new()
    );
}

/// A directory may legally be called `$(id)`. Quoting is what answers that —
/// this function must not drop it, because dropping it costs a real cache on a
/// repository that did nothing wrong.
#[test]
fn a_shell_metacharacter_in_a_name_is_kept_for_the_quoter_to_handle() {
    assert_eq!(
        shareable_cache_paths(&probe(&[
            "sneaky$(id)/target/",
            "weird name/node_modules/",
            "semi;colon/vendor/"
        ])),
        vec![
            "sneaky$(id)/target",
            "weird name/node_modules",
            "semi;colon/vendor"
        ]
    );
}

/// The three hazards quoting does *not* answer, each aimed at a different
/// consumer: the joined roots, the exclude file, and the escaper's own
/// home-directory shortcut.
#[test]
fn a_path_that_could_escape_its_roots_or_its_line_is_refused() {
    for entry in [
        // `cp -R` and `ln -sfn` follow this out of the clone, the cache root
        // and the worktree alike.
        "../../etc/node_modules/",
        "packages/../../../target/",
        "/etc/node_modules/",
        "./node_modules/",
        // `.git/info/exclude` is line-based: everything after the newline
        // becomes a further ignore rule in a file Demeteo does not own.
        "a\nvendor/",
        "node_modules\n*\n",
        "tab\there/target/",
        // `escape_posix` preserves `~` deliberately, so it survives quoting
        // and the shell expands it.
        "~/node_modules/",
        "~root/target/",
        // An empty segment is a path this cannot reason about at all.
        "packages//node_modules/",
    ] {
        assert_eq!(
            shareable_cache_paths(&probe(&[entry])),
            Vec::<String>::new(),
            "{entry:?} must not reach a shell, an exclude file or a join"
        );
    }
}

/// One entry per cache. git lists a directory once, but the answer is also
/// concatenated across `--directory` collapses and a repeat would seed the
/// cache root twice and relink over the first link.
#[test]
fn a_repeated_entry_is_named_once() {
    assert_eq!(
        shareable_cache_paths(&probe(&["target/", "target/", "target"])),
        vec!["target"]
    );
}

/// Nothing is not an error. An empty answer is what a repository with no
/// ignored caches gives, and it is also what a failed probe is flattened to by
/// the caller's `unwrap_or_default`.
#[test]
fn an_empty_answer_names_nothing() {
    assert!(shareable_cache_paths("").is_empty());
    assert!(shareable_cache_paths("\0\0").is_empty());
}
