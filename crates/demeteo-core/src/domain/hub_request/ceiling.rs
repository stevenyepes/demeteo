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
