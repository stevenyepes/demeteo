//! Which of a checkout's ignored directories may be linked into a worktree.
//! See [`crate::domain`].
//!
//! The list this filters used to be the *source* of the paths: a fixed set of
//! top-level basenames, each probed with `[ -e <repo>/<name> ]`. That answered
//! correctly for a repository whose build output sits at the root and silently
//! for every other layout — a Tauri app keeps it at `src-tauri/target`, a JS
//! monorepo at `packages/*/node_modules`, a Gradle build at `*/build`. All of
//! them matched nothing, shared nothing, and reported nothing, so every
//! worktree of those projects rebuilt its whole dependency graph from scratch
//! with a warm one sitting beside it in the clone.
//!
//! Asking git instead — `ls-files --others --ignored --directory` — answers for
//! any depth and any layout, because it is the project's own `.gitignore`
//! talking. What that changes is where the strings come from: they are no
//! longer compiled in, they are whatever the repository contains. So the list
//! stops being the source and becomes the **gate**, and everything below is
//! about what a repository must not be able to say. A directory may legally be
//! named `$(id)`, or hold a newline, and these paths are destined for a shell
//! command, a `.git/info/exclude` line, and three different roots to be joined
//! onto.

use crate::paths::DEPENDENCY_CACHE_DIRS;

/// The shareable caches named by one
/// `git ls-files --others --ignored --directory -z` answer, in the order git
/// gave them and without repeats.
///
/// `-z` is not a detail: git's default output *C-quotes* any path that is not
/// plain ASCII (`"uni\357\277\275code/"`), so the undelimited form cannot be
/// read back without also implementing git's unquoting — and a caller that
/// skips that step hands the quotes themselves to `cp`.
pub fn shareable_cache_paths(probe_output: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in probe_output.split('\0') {
        if let Some(path) = shareable_cache_path(entry) {
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// One entry, cleaned of the trailing `/` git marks a directory with, or `None`
/// when this may not be linked.
///
/// Every rejection below is a thing a repository could otherwise say:
///
/// - **A name that is not a known cache.** git also reports `dist/`,
///   `src-tauri/gen/`, and — when everything beneath them is ignored — the
///   *parent* directories `apps/` and `deep/a/`. Only the final segment is
///   matched, so `packages/web/node_modules` is a cache and `apps/` is not.
///   This is also the rule that keeps generated *source* out: sharing it across
///   a feature's worktrees would make one step's codegen visible to another.
/// - **An absolute path, or one containing `..`.** The path is joined onto the
///   clone, the feature's cache root and the worktree in turn, and `cp -R` and
///   `ln -sfn` follow it out of all three.
/// - **A control character.** `.git/info/exclude` is line-based, so a path
///   carrying a newline appends whatever follows it as a further ignore rule in
///   a file belonging to a repository Demeteo does not own the contents of.
/// - **A leading `~`.** [`escape_posix`](crate::shared::shell::escape_posix)
///   deliberately preserves the home-directory shortcut, so `~` is one of the
///   few things that survives quoting and is then expanded by the shell.
///
/// Shell metacharacters are *not* rejected: `$(id)` is a legal directory name
/// and refusing it would drop a real cache. It is quoted at the call site
/// instead, which is what the escaper is for — this function's job is the
/// hazards quoting does not answer.
pub fn shareable_cache_path(entry: &str) -> Option<String> {
    let path = entry.trim_end_matches('/');
    if path.is_empty() || path.starts_with('/') {
        return None;
    }
    let mut segments = path.split('/').peekable();
    let mut last = None;
    while let Some(segment) = segments.next() {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
        if segment.starts_with('~') || segment.chars().any(char::is_control) {
            return None;
        }
        if segments.peek().is_none() {
            last = Some(segment);
        }
    }
    let last = last?;
    DEPENDENCY_CACHE_DIRS
        .contains(&last)
        .then(|| path.to_string())
}

#[cfg(test)]
#[path = "../../tests/domain/dependency_cache.rs"]
mod tests;
