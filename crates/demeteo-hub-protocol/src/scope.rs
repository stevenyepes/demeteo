use serde::{Deserialize, Serialize};

/// What the user has allowed the Hub to do on this instance.
///
/// The set is flat: `Spend` does not imply `Read`. There is no `logs` scope —
/// transcripts and diffs never leave an instance (HUB.md §3) — and no
/// `configure`, which MCP offers and the Hub does not (HUB.md §7). Nothing in
/// this protocol may enable a scope; one turns on only in the instance's own
/// Settings, so no message type here carries a scope to be granted.
///
/// Deliberately no `#[serde(other)]`: an unknown scope name must fail to parse,
/// not decay into a known one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Read,
    Spend,
    Gates,
}

/// A gate decision relayed by the Hub. `Redirect` exists only because
/// [`gate_challenge`](crate::gate_challenge) binds the feedback it carries: the
/// human's passkey signs the text, so a relaying Hub cannot rewrite it, nor
/// the step a feedback naming one rewinds to (HUB.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Cancel,
    Redirect,
}

/// Reasoning effort for a run. Spellings mirror `demeteo-core`'s `EffortLevel`;
/// the crate does not depend on core to share them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T: serde::de::DeserializeOwned>(name: &str) -> Result<T, serde_json::Error> {
        serde_json::from_str(&format!("\"{name}\""))
    }

    const ALL_SCOPES: [Scope; 3] = [Scope::Read, Scope::Spend, Scope::Gates];

    #[test]
    fn scope_rejects_names_the_protocol_does_not_offer() {
        for name in ["logs", "configure", "Read", ""] {
            assert!(parse::<Scope>(name).is_err(), "{name:?} must not parse");
        }
    }

    #[test]
    fn scope_accepts_exactly_its_three_spellings() {
        assert_eq!(parse::<Scope>("read").unwrap(), Scope::Read);
        assert_eq!(parse::<Scope>("spend").unwrap(), Scope::Spend);
        assert_eq!(parse::<Scope>("gates").unwrap(), Scope::Gates);
    }

    #[test]
    fn scope_serialises_to_the_same_spellings() {
        let names: Vec<String> = ALL_SCOPES
            .iter()
            .map(|s| serde_json::to_string(s).unwrap())
            .collect();
        assert_eq!(names, ["\"read\"", "\"spend\"", "\"gates\""]);
    }

    /// The wildcard-free `match` stops compiling when a variant is added, so a
    /// new scope cannot arrive without this test being looked at.
    #[test]
    fn scope_variant_count_is_fixed_at_three() {
        let count = |s: Scope| match s {
            Scope::Read | Scope::Spend | Scope::Gates => 1,
        };
        assert_eq!(ALL_SCOPES.iter().map(|s| count(*s)).sum::<usize>(), 3);
        assert_eq!(ALL_SCOPES.len(), 3);
    }

    #[test]
    fn decision_parses_exactly_its_three_spellings() {
        assert_eq!(parse::<Decision>("approve").unwrap(), Decision::Approve);
        assert_eq!(parse::<Decision>("cancel").unwrap(), Decision::Cancel);
        assert_eq!(parse::<Decision>("redirect").unwrap(), Decision::Redirect);
        for name in ["reject", "Redirect", ""] {
            assert!(parse::<Decision>(name).is_err(), "{name:?} must not parse");
        }
    }

    #[test]
    fn effort_round_trips_its_five_spellings() {
        let all = [
            (Effort::Low, "low"),
            (Effort::Medium, "medium"),
            (Effort::High, "high"),
            (Effort::Xhigh, "xhigh"),
            (Effort::Max, "max"),
        ];
        for (effort, name) in all {
            assert_eq!(
                serde_json::to_string(&effort).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(parse::<Effort>(name).unwrap(), effort);
        }
        assert!(parse::<Effort>("ultra").is_err());
    }
}
