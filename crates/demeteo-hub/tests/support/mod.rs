//! Shared by the Postgres-backed test binaries.

use demeteo_hub::store::{self, Db};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::future::Future;
use std::str::FromStr;
use uuid::Uuid;

const URL_ENV: &str = "DEMETEO_HUB_TEST_DATABASE_URL";

/// Runs `body` against a schema migrated for it alone and dropped afterwards,
/// so tests run in parallel and leave the database as they found it.
pub async fn in_fresh_schema<F, Fut>(body: F)
where
    F: FnOnce(Db) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let url = std::env::var(URL_ENV)
        .unwrap_or_else(|_| panic!("{URL_ENV} must be set (see deploy/hub/README.md)"));
    let schema = format!("hub_test_{}", Uuid::new_v4().simple());

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();

    // Everything after CREATE SCHEMA runs in the task, so a panic in connect
    // or migrate is caught here too and the DROP below still runs.
    let task_schema = schema.clone();
    let outcome = tokio::spawn(async move {
        let options = PgConnectOptions::from_str(&url)
            .unwrap()
            .options([("search_path", task_schema.as_str())]);
        let db = PgPoolOptions::new().connect_with(options).await.unwrap();
        store::migrate(&db).await.unwrap();
        body(db.clone()).await;
        db.close().await;
    })
    .await;

    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    if let Err(failure) = outcome {
        std::panic::resume_unwind(failure.into_panic());
    }
}
