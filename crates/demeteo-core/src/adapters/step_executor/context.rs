//! Where one step sits in its feature's plan.

use crate::domain::models::{StepConfig, StepExecution};

/// The four values every stage of one step's dispatch passes among
/// themselves: what is running, what it was defined as, and where it sits
/// among its siblings.
///
/// They are already bundled once the dispatch reaches a node handler —
/// [`NodeCtx`](super::registry::NodeCtx) re-collects exactly these four —
/// so spelling them out positionally on the way in was the run loop
/// naming a concept it already had.
///
/// `step_index` and `step_execs` travel with the pair because a
/// `RedirectTo` outcome is expressed as an index into the ordered plan,
/// and resolving it needs the rows.
#[derive(Clone, Copy)]
pub(crate) struct StepCtx<'a> {
    /// The persisted execution row for this step.
    pub step_exec: &'a StepExecution,
    /// The step's definition (v1 model until P1.12 wires v2 through).
    pub step_conf: &'a StepConfig,
    /// Index of this step in the ordered plan.
    pub step_index: usize,
    /// Every step-execution row for the feature, in plan order.
    pub step_execs: &'a [StepExecution],
}
