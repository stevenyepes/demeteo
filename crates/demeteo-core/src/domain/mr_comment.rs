//! What a comment Demeteo posts says about itself. See [`crate::domain`].
//!
//! A review report stops looking like a report the moment it lands on someone
//! else's pull request: the provider renders it under the token owner's name
//! and avatar, in the same column as their colleagues' paragraphs, and nothing
//! in the markup says a machine wrote it. The attribution is the only thing
//! that does, which is why it is appended here — on the way to the provider,
//! for every caller — rather than offered as a field somebody can leave blank.
//! A comment that arrived without it cannot be recalled.

/// Appended verbatim. Markdown, which both providers render in a comment body.
pub const ATTRIBUTION: &str = "*Posted by Demeteo — written by a review agent, not a person.*";

/// The report as it will read on the provider.
pub fn attributed(report: &str) -> String {
    format!("{}\n\n---\n\n{ATTRIBUTION}", report.trim_end())
}

#[cfg(test)]
#[path = "../../tests/domain/mr_comment.rs"]
mod tests;
