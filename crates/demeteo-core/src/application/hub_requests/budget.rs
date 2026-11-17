//! A Hub `set_run_budget` once its id is spent (docs/HUB.md §6.1): the run
//! it names found, its new cap judged against this instance's ceilings, then
//! written through [`HubRunControl::set_run_cap`](crate::ports::hub::HubRunControl::set_run_cap).

use demeteo_hub_protocol::{RefusalReason, SetRunBudget};

use super::{HubRequests, Settled};
use crate::domain::hub_request::{cancel_refusal, cap_change_verdict, HubLocalPolicy};
use crate::domain::ids::FeatureId;

impl HubRequests {
    /// `committed_cents` is the 24-hour total `judged` loaded, under the turn
    /// a start takes, so a raise and a start are never judged against a total
    /// that misses the other.
    pub(super) fn set_budget(
        &self,
        budget: &SetRunBudget,
        committed_cents: Option<u64>,
        policy: &HubLocalPolicy,
        now_ms: u64,
    ) -> Settled {
        if !self.runs.holds_run_caps() {
            return Settled::refused(RefusalReason::UnknownKind);
        }
        let Some(committed) = committed_cents else {
            return Settled::refused(RefusalReason::OverRollingCeiling);
        };
        let feature_id = FeatureId::from(budget.feature_id.clone());
        let current = match self.runs.run_cap_cents(&feature_id) {
            Ok(current) => current,
            Err(e) => {
                tracing::warn!(error = %e, "Hub budget refused: the run's cap did not load");
                return Settled::refused(cancel_refusal(&e));
            }
        };
        let raise = match cap_change_verdict(
            budget.max_budget_cents,
            current,
            policy.run_ceiling_cents,
            committed,
            policy.daily_ceiling_cents,
        ) {
            Ok(raise) => raise,
            Err(reason) => return Settled::refused(reason),
        };
        let now_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
        match self
            .runs
            .set_run_cap(&feature_id, budget.max_budget_cents, raise, now_ms)
        {
            Ok(()) => Settled::accepted(Some(budget.feature_id.clone()), None),
            Err(e) => {
                tracing::warn!(error = %e, "Hub budget refused: the cap was not written");
                Settled::refused(cancel_refusal(&e))
            }
        }
    }
}
