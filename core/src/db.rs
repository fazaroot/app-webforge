use crate::config::Config;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

pub async fn init_pool(config: &Config) -> Result<SqlitePool, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("sqlite://{}?mode=rwc", config.db_path().display());
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    // Honest startup state: processes from a previous run are gone.
    sqlx::query(
        "UPDATE server_instances SET status='stopped', pid=NULL, error_message=NULL, updated_at=?1 WHERE status IN ('running','starting','stopping')",
    )
    .bind(crate::util::now_iso())
    .execute(&pool)
    .await?;
    sqlx::query("DELETE FROM port_allocations").execute(&pool).await?;
    sqlx::query("UPDATE server_processes SET ended_at=?1 WHERE ended_at IS NULL")
        .bind(crate::util::now_iso())
        .execute(&pool)
        .await?;
    crate::state::app_log(&pool, "system", "info", "WebForge core dimulai", None).await;
    Ok(pool)
}
