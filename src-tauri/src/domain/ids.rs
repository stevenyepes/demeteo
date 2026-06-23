//! Newtype wrappers for the various entity IDs that flow through Demeteo.
//!
//! The previous design used `String` everywhere an ID was needed. The cost of
//! that is silent: a `thread_id` accidentally passed where a `feature_id` is
//! expected compiles, runs, and corrupts state. The newtypes below give us
//! compile-time errors for the common mix-ups, with **zero change to the
//! on-the-wire JSON serialization** (each newtype is `#[serde(transparent)]`).
//!
//! Conventions:
//!
//! * Construct from `&str` / `String` via `.into()` or `Id::new(...)`.
//! * Read the inner value via `id.as_str()` or `&*id` (Deref<Target = str>).
//! * When a Tauri command receives an ID from the frontend, it stays a
//!   `String` at the command boundary; the conversion to the newtype happens
//!   at the call site of the port method, not on the wire.
//! * `Display` is implemented so `format!("{}", id)` works.
//!
//! This module is the **single place** that knows the ID vocabulary. If a
//! new entity is added (e.g. `SubtaskId` when the orchestrator needs to
//! reference a subtask), add a new newtype here rather than threading
//! `String` through a new module.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            #[inline]
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            #[inline]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            #[inline]
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Deref for $name {
            type Target = str;
            #[inline]
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            #[inline]
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            #[inline]
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<$name> for String {
            #[inline]
            fn from(id: $name) -> String {
                id.0
            }
        }

        // SQLite support: each ID is stored as a TEXT column. The
        // newtype transparently serialises to/from its inner String.
        impl rusqlite::types::FromSql for $name {
            fn column_result(
                value: rusqlite::types::ValueRef<'_>,
            ) -> rusqlite::types::FromSqlResult<Self> {
                let s = value.as_str()?;
                Ok(Self(s.to_string()))
            }
        }

        impl rusqlite::types::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(rusqlite::types::ToSqlOutput::Borrowed(
                    rusqlite::types::ValueRef::Text(self.0.as_bytes()),
                ))
            }
        }
    };
}

id_newtype!(MachineId);
id_newtype!(ProjectId);
id_newtype!(ThreadId);
id_newtype!(FeatureId);
id_newtype!(WorkflowId);
id_newtype!(StepId);
id_newtype!(StepExecutionId);
id_newtype!(GateDecisionId);
id_newtype!(ProviderId);
id_newtype!(RepositoryId);
id_newtype!(AgentProfileId);
id_newtype!(MessageId);
id_newtype!(WorkflowVersionId);
id_newtype!(InterceptId);

#[cfg(test)]
#[path = "../../tests/domain/ids.rs"]
mod tests;
