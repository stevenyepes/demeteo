use super::build_diff_url;

#[test]
fn build_diff_url_gitlab_with_default_branch() {
    assert_eq!(
        build_diff_url("gitlab", "gitlab.com", "group/repo", "main", "feature/run"),
        "https://gitlab.com/group/repo/-/compare/main...feature/run"
    );
}

#[test]
fn build_diff_url_github_with_default_branch() {
    assert_eq!(
        build_diff_url("github", "github.com", "owner/repo", "main", "feature/run"),
        "https://github.com/owner/repo/compare/main...feature/run"
    );
}

#[test]
fn build_diff_url_gitlab_no_default_branch_falls_back_to_tree() {
    assert_eq!(
        build_diff_url("gitlab", "gitlab.com", "group/repo", "", "feature/run"),
        "https://gitlab.com/group/repo/-/tree/feature/run"
    );
}

#[test]
fn build_diff_url_github_no_default_branch_falls_back_to_tree() {
    assert_eq!(
        build_diff_url("github", "github.com", "owner/repo", " ", "feature/run"),
        "https://github.com/owner/repo/tree/feature/run"
    );
}

#[test]
fn build_diff_url_kind_match_is_case_insensitive() {
    assert_eq!(
        build_diff_url("GiTlAb", "gitlab.example", "group/repo", "main", "branch"),
        "https://gitlab.example/group/repo/-/compare/main...branch"
    );
}
