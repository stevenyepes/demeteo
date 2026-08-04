use serde::{Deserialize, Serialize};

/// User-authored script bodies for the two supported shell families.
///
/// A script is never translated between variants. Local Windows runs the
/// PowerShell body; Unix and remote execution run the POSIX body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptVariants {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub powershell: Option<String>,
}

impl ScriptVariants {
    pub fn is_empty(&self) -> bool {
        self.posix
            .as_deref()
            .is_none_or(|script| script.trim().is_empty())
            && self
                .powershell
                .as_deref()
                .is_none_or(|script| script.trim().is_empty())
    }
}
