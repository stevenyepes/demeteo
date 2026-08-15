use super::*;

mod list;
mod target_branch;

use std::collections::HashMap;
use std::sync::Mutex;

/// An [`HttpClient`] that answers only what it was told to answer.
///
/// It `Err`s on any URL it has no entry for, which is the opposite of the
/// permissive double AGENTS.md §7 warns about: a fake that returns `Ok("")` for
/// everything turns "the adapter called the wrong endpoint" into a passing test
/// asserting a default. Here a wrong URL fails loudly and names itself.
pub struct FakeHttpClient {
    replies: HashMap<String, (u16, String, Vec<(String, String)>)>,
    posted: Mutex<Vec<(String, serde_json::Value)>>,
}

impl FakeHttpClient {
    pub fn new() -> Self {
        Self {
            replies: HashMap::new(),
            posted: Mutex::new(Vec::new()),
        }
    }

    /// The body of the last POST to `url`, which is the only place a caller's
    /// choice of target branch becomes observable: both providers read it from
    /// the payload, and a publisher that dropped it still POSTs to this URL.
    pub fn posted_to(&self, url: &str) -> Option<serde_json::Value> {
        self.posted
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(u, _)| u == url)
            .map(|(_, body)| body.clone())
    }

    /// Answer `url` with this status and body, and no headers.
    pub fn reply(mut self, url: &str, status: u16, body: &str) -> Self {
        self.replies
            .insert(url.to_string(), (status, body.to_string(), Vec::new()));
        self
    }

    pub fn reply_with_headers(
        mut self,
        url: &str,
        status: u16,
        body: &str,
        headers: &[(&str, &str)],
    ) -> Self {
        let headers = headers
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        self.replies
            .insert(url.to_string(), (status, body.to_string(), headers));
        self
    }
}

#[async_trait]
impl HttpClient for FakeHttpClient {
    async fn post_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
        body: &serde_json::Value,
    ) -> Result<HttpResponse, String> {
        self.posted
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone()));
        self.get_json(url, &[]).await
    }

    async fn get_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
    ) -> Result<HttpResponse, String> {
        match self.replies.get(url) {
            Some((status, body, headers)) => Ok(HttpResponse {
                status: *status,
                body: body.clone(),
                headers: headers.clone(),
            }),
            None => Err(format!("FakeHttpClient was never told how to answer {url}")),
        }
    }
}

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
