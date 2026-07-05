//! Filesystem implementation of [`AttachmentStore`].
//!
//! Persists attachments under
//! `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>`.
//! The layout is intentionally flat per feature so
//! [`FsAttachmentStore::clear_feature`] is a single
//! `remove_dir_all` and so the path an agent reads in its worktree
//! resolves cleanly.
//!
//! See `docs/ARCHITECTURE.md` (§2 port catalogue).

use std::path::{Path, PathBuf};

use crate::ports::attachment_store::AttachmentStore;

pub struct FsAttachmentStore {
    root: PathBuf,
}

impl FsAttachmentStore {
    pub fn new(app_local_data_dir: PathBuf) -> Self {
        let root = app_local_data_dir.join("attachments");
        if let Err(e) = std::fs::create_dir_all(&root) {
            tracing::warn!(error = %e, root = %root.display(), "failed to create attachments root");
        }
        Self { root }
    }

    fn feature_dir(&self, feature_id: &str) -> PathBuf {
        self.root.join(sanitize(feature_id))
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl AttachmentStore for FsAttachmentStore {
    fn root(&self) -> &Path {
        &self.root
    }

    fn lookup_path(&self, feature_id: &str, sha256: &str, ext: &str) -> PathBuf {
        self.feature_dir(feature_id)
            .join(format!("{}.{}", sha256, ext))
    }

    fn write(
        &self,
        feature_id: &str,
        sha256: &str,
        ext: &str,
        bytes: &[u8],
    ) -> Result<String, String> {
        let dir = self.feature_dir(feature_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create attachment dir {}: {}", dir.display(), e))?;
        let path = self.lookup_path(feature_id, sha256, ext);
        // Idempotency: a file with the same `<sha256>.<ext>` is
        // already the content we want — leaving the existing file
        // untouched is correct, but we still rewrite if missing so a
        // partial write (e.g. a previous crash) is repaired on retry.
        if path.exists() {
            // Validate that the bytes on disk actually match the hash
            // we were told to write under. If they don't, this is a
            // sha256 collision (extremely unlikely) or a logical bug
            // — log and refuse rather than silently overwrite.
            match std::fs::read(&path) {
                Ok(existing) => {
                    if existing.as_slice() != bytes {
                        tracing::warn!(
                            feature_id = feature_id,
                            sha256 = sha256,
                            existing_len = existing.len(),
                            new_len = bytes.len(),
                            "attachment write found a pre-existing file at the same \
                             <sha256>.<ext> with different bytes; refusing to overwrite \
                             (this would be a sha256 collision or a logical bug)"
                        );
                        return Err(format!(
                            "attachment at {} already exists with different bytes",
                            path.display()
                        ));
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "could not read existing attachment; rewriting");
                    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                }
            }
        } else {
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        }
        Ok(path.to_string_lossy().to_string())
    }

    fn read(&self, stored_path: &str) -> Result<Vec<u8>, String> {
        let p = std::path::Path::new(stored_path);
        // Defense-in-depth: reject paths outside `self.root` so a
        // tampered `stored_path` from the DB can't read elsewhere
        // on disk.
        if !p.starts_with(&self.root) {
            return Err(format!(
                "stored_path {} is outside attachment root {}",
                p.display(),
                self.root.display()
            ));
        }
        std::fs::read(p).map_err(|e| e.to_string())
    }

    fn delete(&self, stored_path: &str) -> Result<(), String> {
        let p = std::path::Path::new(stored_path);
        if !p.starts_with(&self.root) {
            return Err(format!(
                "stored_path {} is outside attachment root {}",
                p.display(),
                self.root.display()
            ));
        }
        if p.exists() {
            std::fs::remove_file(p).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn clear_feature(&self, feature_id: &str) -> Result<(), String> {
        let dir = self.feature_dir(feature_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| {
                format!("failed to remove attachments dir {}: {}", dir.display(), e)
            })?;
        }
        Ok(())
    }

    fn list_for_feature(&self, feature_id: &str) -> Result<Vec<String>, String> {
        let dir = self.feature_dir(feature_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.is_file() {
                out.push(p.to_string_lossy().to_string());
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/attachment_store.rs"]
mod tests;
