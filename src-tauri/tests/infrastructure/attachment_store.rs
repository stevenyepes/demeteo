use crate::adapters::attachment_store::fs::FsAttachmentStore;
use crate::ports::attachment_store::AttachmentStore;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_store() -> (FsAttachmentStore, PathBuf) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "demeteo_attach_test_{}_{}_{}_{}",
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

#[test]
fn write_and_read_round_trip() {
    let (store, dir) = temp_store();
    let bytes = b"hello attachment world";
    let sha = crate::domain::attachment::compute_sha256_hex(bytes);
    let path = store.write("f-1", &sha, "png", bytes).unwrap();
    assert!(path.starts_with(dir.to_string_lossy().as_ref()));
    assert!(path.ends_with(format!("{sha}.png").as_str()));
    let back = store.read(&path).unwrap();
    assert_eq!(back, bytes);
}

#[test]
fn lookup_path_matches_write() {
    let (store, _dir) = temp_store();
    let bytes = b"abc";
    let sha = crate::domain::attachment::compute_sha256_hex(bytes);
    let expected = store.lookup_path("f-2", &sha, "jpg");
    let actual = store.write("f-2", &sha, "jpg", bytes).unwrap();
    assert_eq!(std::path::Path::new(&actual), expected);
}

#[test]
fn write_is_idempotent_for_same_content() {
    let (store, _dir) = temp_store();
    let bytes = b"idempotent content";
    let sha = crate::domain::attachment::compute_sha256_hex(bytes);
    let p1 = store.write("f-3", &sha, "txt", bytes).unwrap();
    let p2 = store.write("f-3", &sha, "txt", bytes).unwrap();
    assert_eq!(p1, p2);
    assert_eq!(store.list_for_feature("f-3").unwrap().len(), 1);
}

#[test]
fn write_refuses_different_bytes_for_existing_path() {
    // Two different payloads that should never collide on sha256 in
    // practice — if this fails, the planet's RNG has betrayed us and
    // the test is more valuable than the assertion.
    let (store, _dir) = temp_store();
    let sha = "0".repeat(64); // place-holder hash
    let _ = store.write("f-4", &sha, "bin", b"first").unwrap();
    let result = store.write("f-4", &sha, "bin", b"second");
    assert!(result.is_err());
}

#[test]
fn clear_feature_drops_everything() {
    let (store, _dir) = temp_store();
    let sha = crate::domain::attachment::compute_sha256_hex(b"x");
    let path = store.write("f-5", &sha, "png", b"x").unwrap();
    assert!(std::path::Path::new(&path).exists());
    store.clear_feature("f-5").unwrap();
    assert!(!std::path::Path::new(&path).exists());
    // Idempotent
    store.clear_feature("f-5").unwrap();
    store.clear_feature("never-existed").unwrap();
}

#[test]
fn list_for_feature_returns_empty_when_absent() {
    let (store, _dir) = temp_store();
    assert!(store.list_for_feature("nope").unwrap().is_empty());
}

#[test]
fn delete_removes_a_single_file() {
    let (store, dir) = temp_store();
    let sha = crate::domain::attachment::compute_sha256_hex(b"y");
    let path = store.write("f-6", &sha, "png", b"y").unwrap();
    assert!(std::path::Path::new(&path).exists());
    store.delete(&path).unwrap();
    assert!(!std::path::Path::new(&path).exists());
    // No-op when missing
    store.delete(&path).unwrap();
    let _ = dir;
}

#[test]
fn read_rejects_paths_outside_root() {
    // Construct a store rooted at /tmp (real fs) and try to read a
    // path that escapes its root. The store must refuse.
    let (store, _dir) = temp_store();
    let bad = "/etc/passwd";
    assert!(store.read(bad).is_err());
}

#[test]
fn delete_rejects_paths_outside_root() {
    let (store, _dir) = temp_store();
    let bad = "/etc/passwd";
    assert!(store.delete(bad).is_err());
}

#[test]
fn list_for_feature_is_sorted() {
    let (store, _dir) = temp_store();
    let s1 = crate::domain::attachment::compute_sha256_hex(b"first");
    let s2 = crate::domain::attachment::compute_sha256_hex(b"second");
    let s3 = crate::domain::attachment::compute_sha256_hex(b"third");
    store.write("f-7", &s2, "png", b"second").unwrap();
    store.write("f-7", &s1, "png", b"first").unwrap();
    store.write("f-7", &s3, "png", b"third").unwrap();
    let listed = store.list_for_feature("f-7").unwrap();
    assert_eq!(listed.len(), 3);
    // sorted ascending
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted);
}
