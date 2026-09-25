//! Re-reading the run's tier-1 assignment pins, once per tick.
//!
//! The pins live on the `features` row and are editable while the run is
//! alive ([`crate::domain::step_assignment`]). An [`ExecutionDriver`] copies
//! them when it arms, so without this read a driver that armed before the
//! edit resolves every remaining node from a list that no longer exists —
//! the pin would appear to take effect only on the next launch.
//!
//! [`ExecutionDriver`]: crate::adapters::step_executor::driver::ExecutionDriver

use crate::domain::ids::FeatureId;
use crate::domain::models::StepOverride;
use crate::ports::db::FeatureRepository;

/// The run's current pins, or `None` for "no change — keep the ones you
/// have".
///
/// Both failure shapes answer `None` rather than an empty list, and the
/// distinction is load-bearing: an empty list *is* the un-pinned state, so
/// returning one after a transient read error or a row the reader raced
/// against deletion would silently drop a pin the user set, and the next
/// dispatch would run the wrong agent. Nothing downstream can tell that
/// apart from a deliberate reset.
///
/// A free function over the one port it reads, not a method on the driver:
/// a driver method is only reachable from a test that first stubs twenty-odd
/// ports it never touches, and a check that expensive to write is a check
/// nobody writes (AGENTS.md §3).
pub(crate) fn refresh_step_overrides(
    features: &dyn FeatureRepository,
    f_id: &FeatureId,
) -> Option<Vec<StepOverride>> {
    match features.get(f_id) {
        Ok(Some(feature)) => Some(feature.step_overrides),
        Ok(None) | Err(_) => None,
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/driver/assignment_refresh.rs"]
mod assignment_refresh_tests;
