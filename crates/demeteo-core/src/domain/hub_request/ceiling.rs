//! The two local ceilings on a Hub-started run (docs/HUB.md §7 *Local
//! ceilings*): a per-run maximum and a rolling 24-hour total, both set on the
//! desktop and both in integer cents.

use demeteo_hub_protocol::RefusalReason;

/// Whether a `start_feature` asking for `cap_cents` fits under both ceilings,
/// given `committed_cents` already committed by Hub launches in the last 24
/// hours.
///
/// Fail closed throughout. The cap is the Hub's number, and a cap the Hub
/// supplies is a cap the Hub can raise, so it bounds nothing until a local
/// ceiling bounds it: an absent cap, an unset run ceiling or an unset daily
/// ceiling each refuse rather than read as "unlimited". Both boundaries are
/// inclusive. The sum is `checked_add`, never wrapping: a cap near `u64::MAX`
/// must not come out as a small total.
pub fn spend_verdict(
    cap_cents: Option<u64>,
    run_ceiling: Option<u64>,
    committed_cents: u64,
    daily_ceiling: Option<u64>,
) -> Result<(), RefusalReason> {
    let (Some(cap), Some(run_ceiling)) = (cap_cents, run_ceiling) else {
        return Err(RefusalReason::OverRunCeiling);
    };
    if cap > run_ceiling {
        return Err(RefusalReason::OverRunCeiling);
    }
    match (committed_cents.checked_add(cap), daily_ceiling) {
        (Some(total), Some(daily)) if total <= daily => Ok(()),
        _ => Err(RefusalReason::OverRollingCeiling),
    }
}

/// Whether a `set_run_budget` may replace a run's cap of `current_cents`
/// (`None`: the run has none) with `new_cents`, and if so how many cents it
/// commits now: the raise, or zero for a cap lowered or kept.
///
/// The new cap is held to the per-run ceiling as a start's is. The rolling
/// ceiling counts the raise alone, at the moment it is made: what the run
/// committed when it started is already in `committed_cents` if it started in
/// the last 24 hours, and spent the window it fell in if it did not. A run
/// with no cap committed nothing, so its new cap is a raise in full. Fail
/// closed for [`spend_verdict`]'s reason, a lowered cap included: a Hub that
/// has no ceiling to be held to sets no cap at all.
pub fn cap_change_verdict(
    new_cents: u64,
    current_cents: Option<u64>,
    run_ceiling: Option<u64>,
    committed_cents: u64,
    daily_ceiling: Option<u64>,
) -> Result<u64, RefusalReason> {
    match run_ceiling {
        Some(run_ceiling) if new_cents <= run_ceiling => {}
        _ => return Err(RefusalReason::OverRunCeiling),
    }
    let raise = new_cents.saturating_sub(current_cents.unwrap_or(0));
    match (committed_cents.checked_add(raise), daily_ceiling) {
        (Some(total), Some(daily)) if total <= daily => Ok(raise),
        _ => Err(RefusalReason::OverRollingCeiling),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_cap_is_held_to_the_run_ceiling_inclusively() {
        assert_eq!(
            cap_change_verdict(1000, Some(900), Some(1000), 0, Some(5000)),
            Ok(100)
        );
        assert_eq!(
            cap_change_verdict(1001, Some(900), Some(1000), 0, Some(5000)),
            Err(RefusalReason::OverRunCeiling)
        );
        assert_eq!(
            cap_change_verdict(500, Some(900), None, 0, Some(5000)),
            Err(RefusalReason::OverRunCeiling)
        );
    }

    #[test]
    fn only_the_raise_counts_against_the_rolling_ceiling() {
        assert_eq!(
            cap_change_verdict(1000, Some(900), Some(1000), 4900, Some(5000)),
            Ok(100)
        );
        assert_eq!(
            cap_change_verdict(1000, Some(900), Some(1000), 4901, Some(5000)),
            Err(RefusalReason::OverRollingCeiling)
        );
        assert_eq!(
            cap_change_verdict(1000, None, Some(1000), 4001, Some(5000)),
            Err(RefusalReason::OverRollingCeiling)
        );
    }

    #[test]
    fn a_lowered_cap_commits_nothing_but_still_needs_both_ceilings() {
        assert_eq!(
            cap_change_verdict(500, Some(900), Some(1000), 5000, Some(5000)),
            Ok(0)
        );
        assert_eq!(
            cap_change_verdict(500, Some(900), Some(1000), 0, None),
            Err(RefusalReason::OverRollingCeiling)
        );
        assert_eq!(
            cap_change_verdict(1, Some(900), Some(1000), u64::MAX, Some(u64::MAX)),
            Ok(0)
        );
    }
}
