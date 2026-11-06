//! Which Feature id a run is reported to the Hub under, and whether this
//! instance reports it at all.
//!
//! The Hub knows a run by the id of the Feature its instance's snapshot
//! lists, and acts on that id: a cancel, a gate decision or an assignment
//! names it. Two instances can hold one run. A desktop that submitted it to
//! a runner keeps a shadow Feature of it, and a runner keys its `run_events`
//! by run id while its snapshot lists the Feature id. So the key a row is
//! logged under is not always the id it is reported under, and on a desktop
//! it is sometimes a run another instance already reports.
//!
//! The desktop leaves a shadow out only once its runner last said it is
//! paired to this desktop's own Hub origin: that runner is then the one
//! instance whose report of the run a Hub request can act on (decision 64).
//! Paired elsewhere, unpaired, never asked, or
//! unreachable since it last answered, and the desktop keeps reporting what
//! it knows, so no run goes unreported. The cost is a short window after a
//! runner pairs in which both report the run, until the next reconcile asks
//! it again.

use serde_json::Value;

use crate::domain::hub_pairing::hub_origin;

/// What a runner last said of its own Hub pairing, as its `hub_status`
/// answered (the `machine_hub_state` table, V69).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerHubState {
    pub paired: bool,
    pub hub_url: Option<String>,
    pub instance_id: Option<String>,
    /// Unix seconds.
    pub checked_at: i64,
}

/// Whose a `run_events` key, or a snapshot Feature id, is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOwner<'a> {
    /// A Feature this instance runs itself.
    Local,
    /// A desktop's shadow of a run on a runner, with what that runner last
    /// said of its pairing; `None` when it was never asked.
    Mirrored { runner: Option<&'a RunnerHubState> },
    /// A run on this runner, with the Feature it bootstrapped, if it got
    /// that far.
    RunnerRun { feature_id: Option<&'a str> },
    /// A runner's key that names neither a run nor a Feature.
    Unknown,
}

/// Whether the runner reports its own runs to the Hub at `own_hub_url`.
/// Origins are compared as [`hub_origin`] normalises them; one that does
/// not normalise matches nothing.
pub fn runner_reports_itself(state: &RunnerHubState, own_hub_url: Option<&str>) -> bool {
    let (Some(theirs), Some(ours)) = (state.hub_url.as_deref(), own_hub_url) else {
        return false;
    };
    state.paired
        && matches!(
            (hub_origin(theirs), hub_origin(ours)),
            (Ok(theirs), Ok(ours)) if theirs == ours
        )
}

/// The Feature id `key` is reported under, or `None` when this instance
/// must not report it.
pub fn reported_feature_id(
    key: &str,
    owner: EventOwner<'_>,
    own_hub_url: Option<&str>,
) -> Option<String> {
    match owner {
        EventOwner::Local => Some(key.to_owned()),
        EventOwner::Mirrored {
            runner: Some(state),
        } if runner_reports_itself(state, own_hub_url) => None,
        EventOwner::Mirrored { .. } => Some(key.to_owned()),
        EventOwner::RunnerRun { feature_id } => feature_id.map(str::to_owned),
        EventOwner::Unknown => None,
    }
}

/// What a runner's `hub_status` answer says of its pairing, read from
/// `status.state`, `status.hub_url` and `status.instance_id` and nothing
/// else; `None` when the answer says nothing, so the last known state
/// stands.
///
/// A runner built before the Hub methods (`unknown method`), or one with no
/// HTTP client for the Hub (`pairing is not available`), is not paired. Any
/// other error is a runner that could not be asked, which is not a runner
/// that unpaired.
pub fn observed_runner_hub_state(
    answer: Result<&Value, &str>,
    checked_at: i64,
) -> Option<RunnerHubState> {
    let status = match answer {
        Ok(value) => value,
        Err(error)
            if error.contains("unknown method") || error.contains("pairing is not available") =>
        {
            &Value::Null
        }
        Err(_) => return None,
    };
    let text = |pointer| status.pointer(pointer)?.as_str().map(str::to_owned);
    Some(RunnerHubState {
        paired: text("/status/state").as_deref() == Some("paired"),
        hub_url: text("/status/hub_url"),
        instance_id: text("/status/instance_id"),
        checked_at,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const OURS: &str = "https://hub.example.com";

    fn state(paired: bool, hub_url: Option<&str>) -> RunnerHubState {
        RunnerHubState {
            paired,
            hub_url: hub_url.map(str::to_owned),
            instance_id: Some("inst-1".to_owned()),
            checked_at: 1,
        }
    }

    #[test]
    fn a_runner_reports_itself_only_when_paired_to_the_same_origin() {
        let cases = [
            (state(true, Some(OURS)), Some(OURS), true),
            (
                state(true, Some("https://HUB.example.com:443/")),
                Some(OURS),
                true,
            ),
            (
                state(true, Some("https://other.example.com")),
                Some(OURS),
                false,
            ),
            (
                state(true, Some("https://hub.example.com:8443")),
                Some(OURS),
                false,
            ),
            (state(false, Some(OURS)), Some(OURS), false),
            (state(true, None), Some(OURS), false),
            (state(true, Some(OURS)), None, false),
            (state(true, Some("not a url")), Some("not a url"), false),
        ];
        for (runner, own, expected) in cases {
            assert_eq!(
                runner_reports_itself(&runner, own),
                expected,
                "{runner:?} against {own:?}"
            );
        }
    }

    #[test]
    fn each_owner_is_reported_under_its_feature_id_or_not_at_all() {
        let same = state(true, Some(OURS));
        let elsewhere = state(true, Some("https://other.example.com"));
        let unpaired = state(false, None);
        let cases: [(EventOwner<'_>, Option<&str>); 8] = [
            (EventOwner::Local, Some("key")),
            (EventOwner::Mirrored { runner: None }, Some("key")),
            (
                EventOwner::Mirrored {
                    runner: Some(&same),
                },
                None,
            ),
            (
                EventOwner::Mirrored {
                    runner: Some(&elsewhere),
                },
                Some("key"),
            ),
            (
                EventOwner::Mirrored {
                    runner: Some(&unpaired),
                },
                Some("key"),
            ),
            (
                EventOwner::RunnerRun {
                    feature_id: Some("f-1"),
                },
                Some("f-1"),
            ),
            (EventOwner::RunnerRun { feature_id: None }, None),
            (EventOwner::Unknown, None),
        ];
        for (owner, expected) in cases {
            assert_eq!(
                reported_feature_id("key", owner, Some(OURS)).as_deref(),
                expected,
                "{owner:?}"
            );
        }
    }

    #[test]
    fn a_hub_status_answer_is_read_from_its_three_fields() {
        let answer = json!({
            "status": {
                "state": "paired",
                "hub_url": OURS,
                "instance_id": "inst-9",
                "fingerprint": "SHA256:ignored",
            },
            "machine": { "hostname": "ignored" },
        });
        assert_eq!(
            observed_runner_hub_state(Ok(&answer), 7),
            Some(RunnerHubState {
                paired: true,
                hub_url: Some(OURS.to_owned()),
                instance_id: Some("inst-9".to_owned()),
                checked_at: 7,
            })
        );
        let incomplete = json!({ "status": { "state": "incomplete", "hub_url": OURS } });
        assert_eq!(
            observed_runner_hub_state(Ok(&incomplete), 7).map(|s| s.paired),
            Some(false)
        );
    }

    #[test]
    fn a_runner_without_pairing_is_unpaired_and_an_unreachable_one_is_unknown() {
        let unpaired = Some(RunnerHubState {
            paired: false,
            hub_url: None,
            instance_id: None,
            checked_at: 7,
        });
        for error in [
            "unknown method: hub_status",
            "runner error: Hub pairing is not available on this runner: it has no HTTP client",
        ] {
            assert_eq!(
                observed_runner_hub_state(Err(error), 7),
                unpaired,
                "{error}"
            );
        }
        assert_eq!(
            observed_runner_hub_state(Err("Timed out waiting on socket"), 7),
            None
        );
    }
}
