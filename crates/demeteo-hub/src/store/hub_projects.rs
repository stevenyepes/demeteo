use super::{StoreError, UserId};
use demeteo_hub_protocol::Effort;
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct HubProjectId(pub Uuid);

impl HubProjectId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Unique per tenant over `(host, full_name)`: a second row naming the same
/// repository is [`StoreError::ProjectExists`].
const REPOSITORY_UNIQUE: &str = "hub_projects_repository";

/// What a tenant writes of a Hub Project, already judged by
/// [`project_verdict`](crate::projects::verdict::project_verdict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubProjectFields {
    pub name: String,
    pub provider_kind: String,
    /// Lowercase, with a port when the clone URL carries one.
    pub host: String,
    pub full_name: String,
    pub default_agent_kind: Option<String>,
    pub default_model: Option<String>,
    pub default_effort: Option<Effort>,
    pub default_workflow_id: Option<String>,
    pub default_max_budget_cents: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubProjectRow {
    pub id: HubProjectId,
    pub fields: HubProjectFields,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(FromRow)]
struct StoredProject {
    id: HubProjectId,
    name: String,
    provider_kind: String,
    host: String,
    full_name: String,
    default_agent_kind: Option<String>,
    default_model: Option<String>,
    default_effort: Option<String>,
    default_workflow_id: Option<String>,
    default_max_budget_cents: Option<i64>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<StoredProject> for HubProjectRow {
    type Error = StoreError;

    fn try_from(row: StoredProject) -> Result<Self, StoreError> {
        let default_effort = row
            .default_effort
            .map(|text| effort_of(&text).ok_or(StoreError::UnknownEffort(text)))
            .transpose()?;
        let default_max_budget_cents = row
            .default_max_budget_cents
            .map(|cents| u64::try_from(cents).map_err(|_| StoreError::BudgetOutOfRange(cents)))
            .transpose()?;
        Ok(Self {
            id: row.id,
            fields: HubProjectFields {
                name: row.name,
                provider_kind: row.provider_kind,
                host: row.host,
                full_name: row.full_name,
                default_agent_kind: row.default_agent_kind,
                default_model: row.default_model,
                default_effort,
                default_workflow_id: row.default_workflow_id,
                default_max_budget_cents,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// The column's spellings, which its `CHECK` lists.
fn effort_text(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
        Effort::Xhigh => "xhigh",
        Effort::Max => "max",
    }
}

fn effort_of(text: &str) -> Option<Effort> {
    [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::Xhigh,
        Effort::Max,
    ]
    .into_iter()
    .find(|effort| effort_text(*effort) == text)
}

fn budget_column(cents: Option<u64>) -> Result<Option<i64>, StoreError> {
    cents
        .map(|cents| i64::try_from(cents).map_err(|_| StoreError::BudgetTooLarge(cents)))
        .transpose()
}

fn write_error(err: sqlx::Error) -> StoreError {
    match err.as_database_error() {
        Some(db) if db.constraint() == Some(REPOSITORY_UNIQUE) => StoreError::ProjectExists,
        _ => StoreError::Database(err),
    }
}

/// A macro and not a `const`, as `store/attachments.rs` says why.
macro_rules! columns {
    () => {
        "id, name, provider_kind, host, full_name, default_agent_kind, default_model, \
         default_effort, default_workflow_id, default_max_budget_cents, created_at, updated_at"
    };
}

const INSERT: &str = concat!(
    "INSERT INTO hub_projects \
     (id, user_id, name, provider_kind, host, full_name, default_agent_kind, \
      default_model, default_effort, default_workflow_id, default_max_budget_cents) \
     VALUES ($2, $1, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
     RETURNING ",
    columns!()
);

const LIST: &str = concat!(
    "SELECT ",
    columns!(),
    " FROM hub_projects WHERE user_id = $1 ORDER BY name, id"
);

const FIND: &str = concat!(
    "SELECT ",
    columns!(),
    " FROM hub_projects WHERE user_id = $1 AND id = $2"
);

const UPDATE: &str = concat!(
    "UPDATE hub_projects SET \
     name = $3, provider_kind = $4, host = $5, full_name = $6, default_agent_kind = $7, \
     default_model = $8, default_effort = $9, default_workflow_id = $10, \
     default_max_budget_cents = $11, updated_at = now() \
     WHERE user_id = $1 AND id = $2 \
     RETURNING ",
    columns!()
);

pub async fn insert_hub_project(
    user: UserId,
    db: impl PgExecutor<'_>,
    id: HubProjectId,
    fields: &HubProjectFields,
) -> Result<HubProjectRow, StoreError> {
    let budget = budget_column(fields.default_max_budget_cents)?;
    sqlx::query_as::<_, StoredProject>(INSERT)
        .bind(user)
        .bind(id)
        .bind(&fields.name)
        .bind(&fields.provider_kind)
        .bind(&fields.host)
        .bind(&fields.full_name)
        .bind(&fields.default_agent_kind)
        .bind(&fields.default_model)
        .bind(fields.default_effort.map(effort_text))
        .bind(&fields.default_workflow_id)
        .bind(budget)
        .fetch_one(db)
        .await
        .map_err(write_error)?
        .try_into()
}

/// Ordered by name, then by id.
pub async fn list_hub_projects(
    user: UserId,
    db: impl PgExecutor<'_>,
) -> Result<Vec<HubProjectRow>, StoreError> {
    sqlx::query_as::<_, StoredProject>(LIST)
        .bind(user)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(HubProjectRow::try_from)
        .collect()
}

pub async fn find_hub_project(
    user: UserId,
    db: impl PgExecutor<'_>,
    id: HubProjectId,
) -> Result<Option<HubProjectRow>, StoreError> {
    sqlx::query_as::<_, StoredProject>(FIND)
        .bind(user)
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(HubProjectRow::try_from)
        .transpose()
}

/// Replaces every field, or `None` when the tenant has no such row.
pub async fn update_hub_project(
    user: UserId,
    db: impl PgExecutor<'_>,
    id: HubProjectId,
    fields: &HubProjectFields,
) -> Result<Option<HubProjectRow>, StoreError> {
    let budget = budget_column(fields.default_max_budget_cents)?;
    sqlx::query_as::<_, StoredProject>(UPDATE)
        .bind(user)
        .bind(id)
        .bind(&fields.name)
        .bind(&fields.provider_kind)
        .bind(&fields.host)
        .bind(&fields.full_name)
        .bind(&fields.default_agent_kind)
        .bind(&fields.default_model)
        .bind(fields.default_effort.map(effort_text))
        .bind(&fields.default_workflow_id)
        .bind(budget)
        .fetch_optional(db)
        .await
        .map_err(write_error)?
        .map(HubProjectRow::try_from)
        .transpose()
}

/// Whether a row was removed. A run already queued or started from it keeps
/// what it was signed with.
pub async fn delete_hub_project(
    user: UserId,
    db: impl PgExecutor<'_>,
    id: HubProjectId,
) -> Result<bool, StoreError> {
    let removed = sqlx::query("DELETE FROM hub_projects WHERE user_id = $1 AND id = $2")
        .bind(user)
        .bind(id)
        .execute(db)
        .await?;
    Ok(removed.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effort_round_trips_through_its_column_spelling() {
        for effort in [
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ] {
            let text = effort_text(effort);
            assert_eq!(serde_json::to_value(effort).unwrap(), text);
            assert_eq!(effort_of(text), Some(effort));
        }
        assert_eq!(effort_of("extreme"), None);
    }

    #[test]
    fn a_budget_past_the_column_is_refused_and_not_clamped() {
        assert_eq!(budget_column(Some(250)).unwrap(), Some(250));
        assert_eq!(budget_column(None).unwrap(), None);
        let err = budget_column(Some(u64::MAX)).unwrap_err();
        assert!(matches!(err, StoreError::BudgetTooLarge(u64::MAX)), "{err}");
    }
}
