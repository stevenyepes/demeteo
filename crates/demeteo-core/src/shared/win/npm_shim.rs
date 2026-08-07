//! Safe direct launches for npm's Windows `.cmd` shims.
//!
//! Rust rejects some arguments to a batch target after CVE-2024-24576. Agent
//! prompts are arbitrary text, so an npm-installed agent must run its
//! `node.exe` and package entrypoint directly rather than transit through
//! `cmd.exe`. This recognises only the fixed path shape npm emits; it never
//! interprets a shim as batch code.

use std::path::{Component, Path, PathBuf};

pub struct NpmShimLaunch {
    pub node: PathBuf,
    pub entrypoint: PathBuf,
}

pub fn direct_launch(
    shim: &Path,
    contents: &str,
    path_node: Option<PathBuf>,
    is_file: &dyn Fn(&Path) -> bool,
) -> Option<NpmShimLaunch> {
    if !shim
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"))
    {
        return None;
    }
    let parent = shim.parent()?;
    let entrypoint = parent.join(npm_entrypoint(contents)?);
    if !is_file(&entrypoint) {
        return None;
    }
    let sibling_node = parent.join("node.exe");
    let node = if is_file(&sibling_node) {
        sibling_node
    } else {
        path_node.filter(|candidate| is_file(candidate))?
    };
    Some(NpmShimLaunch { node, entrypoint })
}

fn npm_entrypoint(contents: &str) -> Option<PathBuf> {
    let marker = "node_modules\\";
    let start = contents.find(marker)?;
    let raw = contents[start..].split('"').next()?.replace('\\', "/");
    let path = PathBuf::from(raw);
    if path
        .extension()
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("js"))
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    Some(path)
}

#[cfg(test)]
#[path = "../../../tests/shared/win/npm_shim.rs"]
mod tests;
