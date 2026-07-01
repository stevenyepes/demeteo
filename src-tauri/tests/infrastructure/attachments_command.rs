//! Tests for the `commands::attachments` module.
//!
//! Included via `#[path = "..."]` from `src/commands/attachments.rs`
//! so the `derive_ext` helper is reachable as `super::derive_ext` (no
//! need to expose it publicly). The `lookup_path` + `read` sequence
//! is exercised against the real `FsAttachmentStore` so the
//! path-within-root safety check that ships in production is the same
//! one under test.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::attachment_store::fs::FsAttachmentStore;
use crate::domain::attachment::{compute_sha256_hex, AttachedFile};
use crate::ports::attachment_store::AttachmentStore;

fn temp_store() -> (FsAttachmentStore, PathBuf) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "demeteo_attach_cmd_test_{}_{}_{}_{}",
        nanos,
        std::process::id(),
        count,
        "store",
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = FsAttachmentStore::new(dir.clone());
    (store, dir)
}

fn make_attached(id: &str, mime: &str, source_filename: &str, bytes: &[u8]) -> AttachedFile {
    AttachedFile {
        id: id.to_string(),
        name: source_filename.to_string(),
        mime: mime.to_string(),
        sha256: compute_sha256_hex(bytes),
        size: bytes.len() as u64,
        source_filename: source_filename.to_string(),
    }
}

/// `derive_ext` is the single source of truth shared by `attachment_read`
/// and `feature_remove_attachment`. The known mime reverse-lookup must
/// win over the source_filename tail.
#[test]
fn derive_ext_prefers_mime_reverse_lookup() {
    // `image/png` → `png`, even though the source filename has `.jpg`.
    assert_eq!(super::derive_ext("image/png", "wrong.jpg"), "png");
    assert_eq!(super::derive_ext("application/pdf", "data.bin"), "pdf");
}

/// Unknown mime falls back to the source_filename tail (lowercased).
#[test]
fn derive_ext_falls_back_to_source_filename_ext() {
    assert_eq!(
        super::derive_ext("application/octet-stream", "notes.TXT"),
        "txt"
    );
    assert_eq!(super::derive_ext("text/x-cargo-lock", "Cargo.lock"), "lock");
}

/// Last-resort fallback is `bin` when there's no usable source tail.
#[test]
fn derive_ext_defaults_to_bin() {
    assert_eq!(
        super::derive_ext("application/octet-stream", "noext"),
        "bin"
    );
    assert_eq!(super::derive_ext("application/octet-stream", ""), "bin");
}

/// `attachment_read` resolves the manifest row by id, derives the ext
/// the same way `feature_remove_attachment` does, then calls
/// `AttachmentStore::lookup_path` + `AttachmentStore::read`. This test
/// exercises that exact sequence against the real store so the
/// path-within-root safety check is the production code, not a mock.
#[test]
fn read_returns_written_bytes_for_known_mime() {
    let (store, _dir) = temp_store();
    let feature_id = "f-read-1";
    let bytes: &[u8] = b"\x89PNG\r\n\x1a\nfakepngbytes";
    let attached = make_attached("at-1", "image/png", "shot.png", bytes);

    let ext = super::derive_ext(&attached.mime, &attached.source_filename);
    assert_eq!(ext, "png");

    let path = store
        .write(feature_id, &attached.sha256, &ext, bytes)
        .expect("store write");
    let stored = store.read(&path).expect("store read");
    assert_eq!(stored, bytes);
}

/// Both ext sources must hit the same on-disk path; if they diverge
/// the read will return different bytes from what was written.
#[test]
fn lookup_path_matches_write_for_derived_ext() {
    let (store, _dir) = temp_store();
    let feature_id = "f-read-3";
    let bytes = b"binary blob \x00\x01\x02";
    let attached = make_attached("at-3", "application/pdf", "report.pdf", bytes);

    let ext = super::derive_ext(&attached.mime, &attached.source_filename);
    let expected = store.lookup_path(feature_id, &attached.sha256, &ext);
    let actual = store
        .write(feature_id, &attached.sha256, &ext, bytes)
        .unwrap();
    assert_eq!(std::path::Path::new(&actual), expected);
}

/// The store's path-within-root check must still fire for the read
/// path the command uses; this is the safety net that prevents a
/// tampered manifest row from reading bytes outside the attachments
/// root.
#[test]
fn read_rejects_paths_outside_root_via_store() {
    let (store, _dir) = temp_store();
    assert!(store.read("/etc/passwd").is_err());
}

// ── is_supported_attachment ───────────────────────────────────────────
//
// The Rust-side file-type allow-list mirrors `ACCEPTED_EXTS` in
// `src/components/AttachmentDropzone.tsx` and `mime_for_ext` in
// `domain::attachment`. The frontend picker uses `accept` as a hint
// only — users can switch the picker to "All Files" and select any
// extension — and Tauri drag-and-drop hands us arbitrary paths with
// no mime. This gate is the second line of defence against uploading
// a non-allowed file type.

#[test]
fn supported_attachment_accepts_known_image_mimes() {
    for mime in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
        assert!(
            super::is_supported_attachment(mime, "png"),
            "{} should be accepted",
            mime
        );
    }
}

#[test]
fn supported_attachment_accepts_known_doc_mimes() {
    for mime in [
        "text/plain",
        "text/markdown",
        "application/json",
        "application/pdf",
    ] {
        assert!(
            super::is_supported_attachment(mime, "bin"),
            "{} should be accepted",
            mime
        );
    }
}

#[test]
fn supported_attachment_rejects_unknown_mimes_and_exts() {
    // Unknown mime + unknown ext → reject.
    assert!(!super::is_supported_attachment(
        "application/octet-stream",
        "exe"
    ));
    assert!(!super::is_supported_attachment("text/x-cargo-lock", "lock"));
    assert!(!super::is_supported_attachment("application/zip", "zip"));
    assert!(!super::is_supported_attachment(
        "application/x-shockwave-flash",
        "swf"
    ));
    // The negative case from the bug report: an `.exe` masquerading as
    // octet-stream must not slip through.
    assert!(!super::is_supported_attachment(
        "application/octet-stream",
        "exe"
    ));
}

#[test]
fn supported_attachment_rejects_unknown_image_mime() {
    // image/svg+xml is not on the allow-list — kept off deliberately
    // to avoid SVG-based prompt-injection vectors. Even though the
    // extension `.svg` could slip through the `image/*` substring
    // check, the explicit allow-list of image/* mimes blocks it.
    assert!(!super::is_supported_attachment("image/svg+xml", "svg"));
}

#[test]
fn supported_extension_fallback_for_known_exts() {
    // Caller supplies a non-IANA mime but a clean extension — still
    // accepted, because the extension is the canonical signal here.
    assert!(super::is_supported_attachment("text/x-patch", "txt"));
    assert!(super::is_supported_attachment("text/x-markdown", "md"));
    assert!(super::is_supported_attachment("application/x-json", "json"));
}
