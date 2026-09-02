//! Join `path` onto `root` and return it only if the result is still `root`
//! or a descendant of it — `None` for anything absolute or that walks out
//! via `..`, the same containment
//! [`crate::domain::models::sandbox::PathContainment`] states for this class
//! of problem elsewhere: a caller's path is not a claim to be trusted, only
//! a claim to be checked against the one root it describes. Shared by every
//! caller that must apply this rule — [`super::turn::verify_canvas_paths`]
//! against a turn's worktree, and click-time resolution against a project's
//! checkout — so the policy is decided in one synchronous place, per
//! `AGENTS.md` §3, rather than re-implemented by hand at each call site.
//!
//! Purely lexical, like [`std::path::Path::join`] itself — no filesystem
//! access, so a `..` past a symlink is not this function's problem, only a
//! `..` past `root`'s own component boundary is.
pub(super) fn resolve_within_root(root: &str, path: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;

    if std::path::Path::new(path).is_absolute() {
        return None;
    }
    let mut components: Vec<Component> = std::path::Path::new(root).components().collect();
    let boundary = components.len();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::Normal(_) => components.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                if components.len() <= boundary {
                    return None;
                }
                components.pop();
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(components.iter().collect())
}

#[cfg(test)]
#[path = "../../../tests/application/ask/path_containment.rs"]
mod tests;
