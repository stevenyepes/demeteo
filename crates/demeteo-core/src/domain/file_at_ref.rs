//! Whether a path names a file at a revision, read out of
//! `git ls-tree -z --full-tree <ref> -- <path>`. See [`crate::domain`].
//!
//! A diff view asks for both sides of every changed path, and a path the range
//! added or deleted is legitimately missing on one side — that side renders as
//! an empty file. Nothing else may render that way: fold every git failure
//! into empty content and a revision this clone has never fetched reads as a
//! file the range deleted, with no error anywhere.
//!
//! `ls-tree` separates the two cases without reading git's prose, which is
//! localised: an unknown revision or a non-repository is a non-zero exit, and
//! a path missing at a revision that *does* resolve is an empty listing.

/// What a revision holds at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAtRef {
    /// The revision resolves and has nothing at the path.
    Absent,
    /// The path is a file; its content is the blob `oid`.
    Blob { oid: String },
}

/// Decide what `listing` — the outcome of the `ls-tree` above, stdout on
/// success and the error on a non-zero exit — says about `path` at `git_ref`.
///
/// Only an entry whose name is exactly `path` counts. A trailing-slash path
/// lists a directory's children instead of the directory, and none of those
/// is the file asked for.
pub fn classify(
    listing: Result<&str, &str>,
    git_ref: &str,
    path: &str,
) -> Result<FileAtRef, String> {
    let listing =
        listing.map_err(|e| format!("cannot read '{path}' at revision '{git_ref}': {e}"))?;

    let Some(entry) = listing
        .split('\0')
        .filter_map(|record| record.split_once('\t'))
        .find(|(_, name)| *name == path)
        .map(|(meta, _)| meta)
    else {
        return Ok(FileAtRef::Absent);
    };

    let mut fields = entry.split(' ');
    let (_mode, kind, oid) = match (fields.next(), fields.next(), fields.next()) {
        (Some(mode), Some(kind), Some(oid)) => (mode, kind, oid),
        _ => {
            return Err(format!(
                "unrecognised git ls-tree entry for '{path}': {entry:?}"
            ))
        }
    };
    if oid.is_empty() || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "unrecognised object id for '{path}' at '{git_ref}': {oid:?}"
        ));
    }
    match kind {
        "blob" => Ok(FileAtRef::Blob {
            oid: oid.to_string(),
        }),
        "tree" => Err(format!("'{path}' is a directory at revision '{git_ref}'")),
        "commit" => Err(format!("'{path}' is a submodule at revision '{git_ref}'")),
        other => Err(format!(
            "'{path}' is a git {other} at revision '{git_ref}', not a file"
        )),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/file_at_ref.rs"]
mod tests;
