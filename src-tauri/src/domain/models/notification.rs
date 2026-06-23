use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Categorical tag for a [`Notification`]. The wire form of the
/// `kind` column on the `notifications` table and the `kind` tag on
/// the in-process `DomainEvent::MrMerged` family. Adding a variant
/// here requires a new `match` arm in the Tauri notification adapter
/// (for the live event) and in the SQL adapter (for the persisted
/// column round-trip) — keep the set small and intentional.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// `MrPublisher` reported the MR has been merged on the
    /// provider. Emitted by the background MR-state monitor.
    MrMerged,
    /// A `gate` step transitioned to `awaiting_gate`. The UI should
    /// surface this so the user knows to act.
    GatePending,
    /// A `StepExecution` transitioned to `failed`.
    StepFailed,
    /// `Feature.status` transitioned to `completed`.
    FeatureCompleted,
    /// `MergeExecutor` reported a conflict between two subtask
    /// branches (or a feature-upstream sync).
    MergeConflict,
}

impl NotificationKind {
    /// Wire form: the `kind` column in `notifications` and the
    /// variant tag in JSON. Stable across releases.
    pub fn as_str(&self) -> &'static str {
        match self {
            NotificationKind::MrMerged => "mr_merged",
            NotificationKind::GatePending => "gate_pending",
            NotificationKind::StepFailed => "step_failed",
            NotificationKind::FeatureCompleted => "feature_completed",
            NotificationKind::MergeConflict => "merge_conflict",
        }
    }
}

impl FromStr for NotificationKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mr_merged" => Ok(NotificationKind::MrMerged),
            "gate_pending" => Ok(NotificationKind::GatePending),
            "step_failed" => Ok(NotificationKind::StepFailed),
            "feature_completed" => Ok(NotificationKind::FeatureCompleted),
            "merge_conflict" => Ok(NotificationKind::MergeConflict),
            _ => Err(()),
        }
    }
}

/// A user-visible event rendered by the notification bell.
///
/// `message` is pre-formatted human text — the bell doesn't try
/// to localize or templatize from structured fields, so the writer
/// (the MR monitor, gate step handler, etc.) is responsible for
/// phrasing. `feature_url` is a relative deep link the UI can
/// follow when the user clicks the row.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Notification {
    pub id: String,
    pub project_id: String,
    pub feature_id: String,
    pub kind: NotificationKind,
    pub message: String,
    pub feature_url: Option<String>,
    pub read: bool,
    pub created_at: i64,
}
