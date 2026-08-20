//! How a project says a review should be started.
//!
//! Layer 1 of the three the review starter leans on, and the only one Demeteo
//! stores: the project names a command (`/code-review`, `/review --deep`, a
//! sentence naming a skill), and the prompt carries it through untouched. Layer
//! 2 is the starter's one-sentence invitation to use whatever review capability
//! the harness has, and layer 3 is the conventions file the harness reads on its
//! own. Precedence runs 1 → 2 → 3, and the layers below need no code here: they
//! are what the prompt already says when this one is empty.
//!
//! ## Why the value is passed through verbatim
//!
//! Every harness spells its own entrypoint differently and none publishes the
//! list, so there is nothing to validate against and nothing to translate into.
//! The moment Demeteo wraps the value — "Start by running X", "The project
//! prefers X" — it has authored review vocabulary of its own, which is the one
//! thing this workflow exists not to do: the report then reflects Demeteo's idea
//! of a review rather than the project's. Same reasoning as `harnesses`, where
//! the user names the command and the orchestrator only runs it.
//!
//! Unset is therefore rendered as nothing at all, not as a heading with nothing
//! under it. A step handed an empty section reads it as an instruction that was
//! meant to say something, and a reviewer told to review "" is worse off than
//! one told nothing.

/// What `{{review_entrypoint}}` binds to, given the project's setting.
///
/// `None` and whitespace-only both mean the project named none — a cleared text
/// input persists as `""`, and a second spelling of unset would render as a
/// blank line where nothing belongs.
pub fn review_entrypoint_binding(configured: Option<&str>) -> &str {
    match configured.map(str::trim) {
        Some(entrypoint) if !entrypoint.is_empty() => entrypoint,
        _ => "",
    }
}

#[cfg(test)]
#[path = "../../tests/domain/review_entrypoint.rs"]
mod tests;
