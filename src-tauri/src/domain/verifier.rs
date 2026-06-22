use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifierConfig {
    /// Agent kind for the verifier. `None` = same as the step's agent_kind.
    pub agent_kind: Option<String>,
    /// Instructions injected as the verifier's prompt preamble.
    pub instructions: String,
    /// Name of the harness to run (e.g. "lint", "integration"). If `None`, falls back to the project's default `test_command`.
    pub harness_name: Option<String>,
    /// JSON key whose value must be `"pass"` or `"fail"`. Default: `"verdict"`.
    #[serde(default = "default_verdict_key")]
    pub verdict_key: String,
}

fn default_verdict_key() -> String {
    "verdict".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierVerdict {
    Pass,
    Fail(String), // reason
}
