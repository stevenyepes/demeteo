use super::*;

#[test]
fn urlencoded_handles_slashes() {
    assert_eq!(urlencoded("owner/repo"), "owner%2Frepo");
    assert_eq!(urlencoded("group/sub/proj"), "group%2Fsub%2Fproj");
    assert_eq!(urlencoded("plain"), "plain");
    assert_eq!(urlencoded("with space"), "with%20space");
}

#[test]
fn extract_number_from_github_url() {
    assert_eq!(
        extract_number_from_url("https://api.github.com/repos/o/r/pulls/42"),
        Some(42)
    );
    assert_eq!(
        extract_number_from_url("https://gitlab.com/g/p/-/merge_requests/7"),
        Some(7)
    );
    assert_eq!(extract_number_from_url("https://example.com/"), None);
}

#[test]
fn feature_id_to_branch_returns_feature_id() {
    let fid = FeatureId::from("f-12345");
    let branch = feature_id_to_branch("any title", &fid);
    assert_eq!(branch, "f-12345");
}
