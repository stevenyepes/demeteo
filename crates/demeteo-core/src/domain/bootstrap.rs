//! State machine for the "create a project from zero" wizard.
//!
//! The wizard is a routed `AppView::CreateProject` view (see
//! `crate::domain::app_view`). It guides the user through seven
//! one-decision-per-screen steps, in a fixed order, then auto-launches
//! `wf-starter-standard` against the freshly-created repo.
//!
//! ## Step ordering — locked
//!
//! `STEP_ORDER` is the canonical, compile-time-enforced order. Adding
//! or reordering a step is a UX-breaking change on the React side and
//! must coordinate with `src/components/wizard/*Step.tsx`. **No**
//! strategy, workflow, or launching screen sits between the seven
//! core steps — those concerns are handled after the wizard completes
//! by `create_project → bootstrap_project → save_project_settings →
//! start_feature`, not inside it.
//!
//! ## History vs current step
//!
//! The wizard can sometimes advance past a step without a user
//! decision (e.g. only one provider configured → skip the Provider
//! screen). Such auto-progressions are still appended to `history`
//! so the wizard retains a complete chronological record and never
//! silently re-enters an auto-progressed screen on `goBack`.
//! `last_user_visible_step` returns the entry immediately preceding
//! the current step — i.e. the step the wizard was on before the
//! most recent transition. The wizard frontend is responsible for
//! filtering out auto-progressed entries if it wants to skip past
//! them; the domain's job is to expose the raw history.

use serde::{Deserialize, Serialize};

/// One decision screen of the create-from-zero wizard.
///
/// The variant order is fixed (see [`STEP_ORDER`]) and **must not**
/// change without coordinating with the matching React step
/// components. Adding a variant is acceptable only if it slots in at
/// the end (after `Description`) and the wizard frontend is updated
/// in lock-step; reordering breaks the user's mental model and any
/// persisted `BootstrapState` (none today, but the door is open).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BootstrapStep {
    /// Repository slug (validated against provider rules).
    Name,
    /// Connected provider (github / gitlab) — auto-progressed if only one is configured.
    Provider,
    /// Namespace (personal account, org, or group) — auto-progressed when only one exists.
    Group,
    /// Local vs remote machine that owns the worktree.
    Machine,
    /// Coding agent kind (opencode / hermes / claude-code).
    Agent,
    /// Model identifier. Picker disabled until both `Machine` and `Agent` are set.
    Model,
    /// Free-text feature description + repo creation confirmation.
    Description,
}

impl BootstrapStep {
    /// Wire form of the variant (kebab-case). Stable across releases —
    /// the React step components import the matching kebab-case slug.
    pub fn as_str(&self) -> &'static str {
        match self {
            BootstrapStep::Name => "name",
            BootstrapStep::Provider => "provider",
            BootstrapStep::Group => "group",
            BootstrapStep::Machine => "machine",
            BootstrapStep::Agent => "agent",
            BootstrapStep::Model => "model",
            BootstrapStep::Description => "description",
        }
    }

    /// Index of `self` in [`STEP_ORDER`]. Constant-time.
    pub fn index(self) -> usize {
        Self::index_of(self)
    }

    /// Index of any step in [`STEP_ORDER`]. Returns `usize::MAX` for
    /// values that aren't in the order (impossible for well-formed
    /// inputs, but defensive).
    pub fn index_of(step: BootstrapStep) -> usize {
        STEP_ORDER
            .iter()
            .position(|s| *s == step)
            .unwrap_or(usize::MAX)
    }

    /// Next step in `STEP_ORDER`, or `None` if `self` is the last one.
    pub fn next(self) -> Option<BootstrapStep> {
        let idx = self.index();
        STEP_ORDER.get(idx + 1).copied()
    }

    /// Previous step in `STEP_ORDER`, or `None` if `self` is the first one.
    pub fn previous(self) -> Option<BootstrapStep> {
        let idx = self.index();
        if idx == 0 {
            None
        } else {
            STEP_ORDER.get(idx - 1).copied()
        }
    }
}

/// Canonical ordering of the seven wizard steps. Locked at exactly
/// seven entries — no strategy, workflow, or launching screen sits in
/// the wizard; those concerns are handled by `create_project →
/// bootstrap_project → save_project_settings → start_feature` after
/// the wizard completes.
///
/// Index in this slice **is** the step's display position in the UI's
/// progress indicator.
pub const STEP_ORDER: [BootstrapStep; 7] = [
    BootstrapStep::Name,
    BootstrapStep::Provider,
    BootstrapStep::Group,
    BootstrapStep::Machine,
    BootstrapStep::Agent,
    BootstrapStep::Model,
    BootstrapStep::Description,
];

/// `BootstrapState` is the wizard's in-memory state.
///
/// `step` is the screen the user is currently on. `history` is the
/// ordered, append-only log of every step the wizard transitioned to
/// (including ones the user auto-progressed past), so [`goBack`] can
/// rewind to the exact previous position without re-entering an
/// auto-progressed screen.
///
/// [`goBack`]: BootstrapState::go_back
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapState {
    /// The step the user is currently on. Invariant: `step ==
    /// history.last()` once the wizard has been initialised.
    pub step: BootstrapStep,
    /// Chronological log of every step the wizard was on, including
    /// the current one. Auto-progressed steps are appended so the
    /// wizard never loses its place when the user goes back.
    pub history: Vec<BootstrapStep>,
}

impl Default for BootstrapState {
    fn default() -> Self {
        Self::new()
    }
}

impl BootstrapState {
    /// Initial state: user is on the first step (`Name`), with a
    /// single-entry history.
    pub fn new() -> Self {
        Self {
            step: BootstrapStep::Name,
            history: vec![BootstrapStep::Name],
        }
    }

    /// Initial state parked on an explicit step. Useful for tests and
    /// for restoring a wizard session from persisted state.
    pub fn starting_at(step: BootstrapStep) -> Self {
        Self {
            step,
            history: vec![step],
        }
    }

    /// Append a transition to `next` and update the current step.
    /// `next` is pushed onto `history` even if it equals `self.step`
    /// (so the wizard can record a re-entry of the same screen).
    pub fn advance_to(&mut self, next: BootstrapStep) {
        self.history.push(next);
        self.step = next;
    }

    /// Pop the current step off the history and rewind to whatever
    /// was on screen before it. Returns `true` when a step was
    /// rewound, `false` when the wizard was already on its first step
    /// (i.e. there was nothing to go back to — the caller should
    /// dismiss the wizard or no-op).
    ///
    /// This implementation does **not** silently swallow
    /// auto-progressed entries between the current step and the next
    /// user-visible one. The wizard frontend is expected to inspect
    /// the resulting `step` and, if it knows the screen was
    /// auto-progressed, call `go_back` again to keep rewinding. That
    /// keeps the domain pure (no "auto-progressed" flag) while still
    /// letting the wizard skip past transient screens.
    pub fn go_back(&mut self) -> bool {
        if self.history.len() <= 1 {
            return false;
        }
        self.history.pop();
        self.step = *self.history.last().expect("history non-empty");
        true
    }

    /// True when the wizard has at least one prior step to rewind to.
    pub fn can_go_back(&self) -> bool {
        self.history.len() > 1
    }

    /// Position of the current step in [`STEP_ORDER`]. `0` for `Name`,
    /// `6` for `Description`.
    pub fn step_index(&self) -> usize {
        BootstrapStep::index_of(self.step)
    }

    /// The step the wizard was on immediately before the current one,
    /// regardless of whether that step was user-driven or
    /// auto-progressed. Returns `None` when the wizard is on its
    /// first step.
    ///
    /// "User-visible" here means "the step recorded just before the
    /// current one in `history`" — i.e. the screen the wizard
    /// displayed before transitioning to where it is now. Auto-
    /// progressed entries are still returned; the wizard frontend is
    /// responsible for filtering them out if it wants to skip past
    /// transient screens on `goBack`.
    pub fn last_user_visible_step(&self) -> Option<BootstrapStep> {
        if self.history.len() < 2 {
            return None;
        }
        self.history.get(self.history.len() - 2).copied()
    }

    /// `true` if the wizard is on the final step (Description).
    pub fn is_final_step(&self) -> bool {
        self.step == BootstrapStep::Description
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_order_has_exactly_seven_entries() {
        // Locked: no strategy / workflow / launching screen sits in
        // the wizard. If you find yourself tempted to extend this,
        // add the screen *after* the wizard completes instead.
        assert_eq!(STEP_ORDER.len(), 7);
    }

    #[test]
    fn step_order_is_documented_sequence() {
        assert_eq!(
            STEP_ORDER,
            [
                BootstrapStep::Name,
                BootstrapStep::Provider,
                BootstrapStep::Group,
                BootstrapStep::Machine,
                BootstrapStep::Agent,
                BootstrapStep::Model,
                BootstrapStep::Description,
            ]
        );
    }

    #[test]
    fn step_order_serialises_as_expected_kebab_strings() {
        let rendered: Vec<&str> = STEP_ORDER.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            rendered,
            [
                "name",
                "provider",
                "group",
                "machine",
                "agent",
                "model",
                "description"
            ]
        );
    }

    #[test]
    fn step_indices_are_zero_through_six() {
        for (i, step) in STEP_ORDER.iter().enumerate() {
            assert_eq!(step.index(), i, "step {step:?} should have index {i}");
            assert_eq!(BootstrapStep::index_of(*step), i);
        }
    }

    #[test]
    fn first_step_has_no_previous() {
        assert_eq!(BootstrapStep::Name.previous(), None);
        assert_eq!(BootstrapStep::Name.next(), Some(BootstrapStep::Provider));
    }

    #[test]
    fn last_step_has_no_next() {
        assert_eq!(BootstrapStep::Description.next(), None);
        assert_eq!(
            BootstrapStep::Description.previous(),
            Some(BootstrapStep::Model)
        );
    }

    #[test]
    fn mid_step_previous_and_next_round_trip() {
        for (i, step) in STEP_ORDER.iter().enumerate() {
            if i > 0 {
                assert_eq!(step.previous(), Some(STEP_ORDER[i - 1]));
            }
            if i + 1 < STEP_ORDER.len() {
                assert_eq!(step.next(), Some(STEP_ORDER[i + 1]));
            }
        }
    }

    #[test]
    fn new_state_is_parked_on_name_with_single_entry_history() {
        let s = BootstrapState::new();
        assert_eq!(s.step, BootstrapStep::Name);
        assert_eq!(s.history, vec![BootstrapStep::Name]);
        assert_eq!(s.step_index(), 0);
        assert!(!s.can_go_back());
        assert_eq!(s.last_user_visible_step(), None);
        assert!(!s.is_final_step());
    }

    #[test]
    fn advance_to_pushes_step_and_updates_current() {
        let mut s = BootstrapState::new();
        s.advance_to(BootstrapStep::Provider);
        assert_eq!(s.step, BootstrapStep::Provider);
        assert_eq!(
            s.history,
            vec![BootstrapStep::Name, BootstrapStep::Provider]
        );
        assert_eq!(s.step_index(), 1);
        assert!(s.can_go_back());
    }

    #[test]
    fn advance_through_full_sequence_records_every_step() {
        let mut s = BootstrapState::new();
        for next in STEP_ORDER.iter().copied().skip(1) {
            s.advance_to(next);
        }
        assert_eq!(s.step, BootstrapStep::Description);
        assert_eq!(s.step_index(), 6);
        assert!(s.is_final_step());
        assert_eq!(s.history, STEP_ORDER.to_vec());
    }

    #[test]
    fn can_go_back_is_false_on_initial_step_and_true_after_one_advance() {
        let mut s = BootstrapState::new();
        assert!(!s.can_go_back());
        s.advance_to(BootstrapStep::Provider);
        assert!(s.can_go_back());
    }

    #[test]
    fn last_user_visible_step_returns_previous_entry() {
        let mut s = BootstrapState::new();
        assert_eq!(s.last_user_visible_step(), None);
        s.advance_to(BootstrapStep::Provider);
        assert_eq!(s.last_user_visible_step(), Some(BootstrapStep::Name));
        s.advance_to(BootstrapStep::Group);
        assert_eq!(s.last_user_visible_step(), Some(BootstrapStep::Provider));
    }

    #[test]
    fn history_records_auto_progressed_steps() {
        // When the wizard auto-progresses past a screen (e.g. only
        // one provider configured), the auto-progressed step must
        // still appear in `history` so `goBack` cannot accidentally
        // jump past it.
        let mut s = BootstrapState::new();
        s.advance_to(BootstrapStep::Provider);
        s.advance_to(BootstrapStep::Group); // assume this is auto-progressed
        s.advance_to(BootstrapStep::Machine);

        assert_eq!(s.step, BootstrapStep::Machine);
        assert_eq!(
            s.history,
            vec![
                BootstrapStep::Name,
                BootstrapStep::Provider,
                BootstrapStep::Group,
                BootstrapStep::Machine,
            ]
        );
        // Group is still in history; if the wizard frontend wants to
        // skip past it on goBack, it can call go_back() and check
        // whether the new step is one it considers auto-progressed.
        assert_eq!(
            s.last_user_visible_step(),
            Some(BootstrapStep::Group),
            "auto-progressed steps must be returned, not filtered out"
        );
    }

    #[test]
    fn go_back_rewinds_to_previous_history_entry() {
        let mut s = BootstrapState::new();
        s.advance_to(BootstrapStep::Provider);
        s.advance_to(BootstrapStep::Group);
        assert!(s.go_back());
        assert_eq!(s.step, BootstrapStep::Provider);
        assert_eq!(
            s.history,
            vec![BootstrapStep::Name, BootstrapStep::Provider]
        );
        assert!(s.can_go_back());
    }

    #[test]
    fn go_back_returns_false_on_initial_step() {
        let mut s = BootstrapState::new();
        assert!(!s.go_back());
        assert_eq!(s.step, BootstrapStep::Name);
        assert_eq!(s.history, vec![BootstrapStep::Name]);
    }

    #[test]
    fn go_back_through_auto_progressed_chain_lands_on_user_step() {
        // history: [Name, Provider(auto), Group(auto), Machine]
        // goBack → Group(auto) → wizard frontend sees auto-progressed
        // and calls goBack again → Provider(auto) → again → Name.
        let mut s = BootstrapState::new();
        s.advance_to(BootstrapStep::Provider);
        s.advance_to(BootstrapStep::Group);
        s.advance_to(BootstrapStep::Machine);

        assert!(s.go_back());
        assert_eq!(s.step, BootstrapStep::Group);
        assert!(s.go_back());
        assert_eq!(s.step, BootstrapStep::Provider);
        assert!(s.go_back());
        assert_eq!(s.step, BootstrapStep::Name);
        // Now at the initial step — further goBack is a no-op.
        assert!(!s.can_go_back());
        assert!(!s.go_back());
        assert_eq!(s.step, BootstrapStep::Name);
    }

    #[test]
    fn starting_at_sets_history_to_single_entry() {
        let s = BootstrapState::starting_at(BootstrapStep::Machine);
        assert_eq!(s.step, BootstrapStep::Machine);
        assert_eq!(s.history, vec![BootstrapStep::Machine]);
        assert_eq!(s.step_index(), 3);
        assert!(!s.can_go_back());
    }

    #[test]
    fn starting_at_final_step_marks_final() {
        let s = BootstrapState::starting_at(BootstrapStep::Description);
        assert!(s.is_final_step());
    }

    #[test]
    fn default_impl_matches_new() {
        let a = BootstrapState::default();
        let b = BootstrapState::new();
        assert_eq!(a, b);
    }

    #[test]
    fn serde_round_trips_state_with_history() {
        let mut s = BootstrapState::new();
        s.advance_to(BootstrapStep::Provider);
        s.advance_to(BootstrapStep::Machine);
        let json = serde_json::to_string(&s).unwrap();
        let back: BootstrapState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.history, s.history);
        assert_eq!(back.step, s.step);
    }
}
