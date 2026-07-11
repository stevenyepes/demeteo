// Tests extracted from `crates/demeteo-core/src/domain/attachment.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn sanitize_strips_path_separators() {
    // Each non-alphanumeric, non-`-`/`_` char becomes `_`.
    // `..` → `__`, `/` → `_`, `\..evil.txt` → `_` + alphanumeric + `_txt`
    assert_eq!(
        sanitize_attachment_filename("../etc/passwd"),
        "___etc_passwd"
    );
    assert_eq!(sanitize_attachment_filename("..\\evil.txt"), "___evil_txt");
}

#[test]
fn sanitize_strips_null_bytes() {
    assert_eq!(sanitize_attachment_filename("name\0.png"), "name__png");
}

#[test]
fn sanitize_keeps_unicode() {
    // Unicode chars are mapped to '_' (non-ascii alphanumeric).
    assert_eq!(sanitize_attachment_filename("café.png"), "caf__png");
}

#[test]
fn sanitize_handles_empty_input() {
    assert_eq!(sanitize_attachment_filename(""), "attachment");
    assert_eq!(sanitize_attachment_filename("..."), "___");
    // Leading dash replaced
    assert!(sanitize_attachment_filename("-evil").starts_with('_'));
}

#[test]
fn mime_for_ext_known_types() {
    assert_eq!(mime_for_ext("png"), Some("image/png"));
    assert_eq!(mime_for_ext("JPG"), Some("image/jpeg"));
    assert_eq!(mime_for_ext("xyz"), None);
}

#[test]
fn compute_sha256_known_vector() {
    // "abc" → ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
    let hex = compute_sha256_hex(b"abc");
    assert_eq!(
        hex,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn compute_sha256_empty_input() {
    // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    let hex = compute_sha256_hex(b"");
    assert_eq!(
        hex,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn compute_sha256_long_input() {
    // Million-a test: compute SHA-256 over a million 'a' bytes.
    // Verified against Python's hashlib (NIST reference).
    let buf = vec![b'a'; 1_000_000];
    let hex = compute_sha256_hex(&buf);
    assert_eq!(
        hex,
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn attached_file_on_disk_path() {
    let f = AttachedFile {
        id: "at-1".into(),
        name: "shot".into(),
        mime: "image/png".into(),
        sha256: "abc123".into(),
        size: 42,
        source_filename: "shot.png".into(),
    };
    assert_eq!(
        f.on_disk_path(std::path::Path::new("/tmp/att"), "png"),
        std::path::PathBuf::from("/tmp/att/abc123.png")
    );
}

#[test]
fn attached_file_serde_roundtrip() {
    let f = AttachedFile {
        id: "at-1".into(),
        name: "shot".into(),
        mime: "image/png".into(),
        sha256: "abc123".into(),
        size: 42,
        source_filename: "shot.png".into(),
    };
    let json = serde_json::to_string(&f).unwrap();
    let back: AttachedFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back, f);
}
