use crate::ports::git_credentials::GitCredentialSource;
use std::sync::{Arc, Mutex};

/// A credential source that answers only for the repo dirs it was told about
/// and errors for every other, so a call site that hands over the wrong
/// directory comes back unauthenticated and fails its test loudly.
pub struct FakeCredentialSource {
    known: Vec<(String, String)>,
    asked: Mutex<Vec<(String, String)>>,
}

impl FakeCredentialSource {
    pub fn new(known: &[(&str, &str)]) -> Arc<Self> {
        Arc::new(Self {
            known: known
                .iter()
                .map(|(dir, pat)| (dir.to_string(), pat.to_string()))
                .collect(),
            asked: Mutex::new(Vec::new()),
        })
    }

    /// Every `(provider_id, repo_dir)` pair the source was asked for, in order.
    pub fn asked(&self) -> Vec<(String, String)> {
        self.asked.lock().unwrap().clone()
    }
}

impl GitCredentialSource for FakeCredentialSource {
    fn pat_for(&self, provider_id: &str, repo_dir: &str) -> Result<String, String> {
        self.asked
            .lock()
            .unwrap()
            .push((provider_id.to_string(), repo_dir.to_string()));
        self.known
            .iter()
            .find(|(dir, _)| dir == repo_dir)
            .map(|(_, pat)| pat.clone())
            .ok_or_else(|| format!("no credential told for {repo_dir}"))
    }
}
