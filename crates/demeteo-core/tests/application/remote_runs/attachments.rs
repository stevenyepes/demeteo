use super::{attachment_spool_dir, MAX_DETACHED_ATTACHMENT_BYTES};

#[test]
fn attachment_spool_dir_returns_xdg_under_home() {
    assert_eq!(
        attachment_spool_dir("/home/alice", "laptop-1"),
        "/home/alice/.local/share/demeteo-runner/attachment-spool/laptop-1"
    );
    assert_eq!(MAX_DETACHED_ATTACHMENT_BYTES, 25 * 1024 * 1024);
}

#[test]
fn attachment_spool_dir_uses_run_id() {
    let first = attachment_spool_dir("/home/alice", "laptop-1");
    let second = attachment_spool_dir("/home/alice", "laptop-2");
    assert!(first.ends_with("/laptop-1"));
    assert!(second.ends_with("/laptop-2"));
    assert_ne!(first, second);
}
