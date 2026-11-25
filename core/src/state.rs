use crate::config::Config;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::process::Child;
use tokio::sync::Mutex;

pub enum ProcessHandle {
    Child(Arc<Mutex<Option<Child>>>),
    InProcess(tokio::task::AbortHandle),
}

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Config,
    pub started_at: DateTime<Utc>,
    pub started_instant: Instant,
    pub processes: Arc<Mutex<HashMap<String, ProcessHandle>>>,
}

impl AppState {
    pub fn uptime_seconds(&self) -> i64 {
        self.started_instant.elapsed().as_secs() as i64
    }
}

pub async fn app_log(pool: &SqlitePool, category: &str, severity: &str, message: &str, server_id: Option<&str>) {
    let _ = sqlx::query(
        "INSERT INTO application_logs (id, category, severity, message, server_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(crate::util::new_id())
    .bind(category)
    .bind(severity)
    .bind(message)
    .bind(server_id)
    .bind(crate::util::now_iso())
    .execute(pool)
    .await;
}
