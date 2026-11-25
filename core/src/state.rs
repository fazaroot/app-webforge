use crate::config::Config;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::process::Child;
use tokio::sync::Mutex;

/// Path nativeLibraryDir Android, diisi lewat JNI nativeSetNativeLibDir
/// sebelum core start. Di host/test tetap None (deteksi PHP memakai PATH).
static NATIVE_LIB_DIR: OnceLock<std::sync::Mutex<Option<PathBuf>>> = OnceLock::new();

pub fn set_native_lib_dir(dir: PathBuf) {
    let slot = NATIVE_LIB_DIR.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(dir);
    }
}

pub fn native_lib_dir() -> Option<PathBuf> {
    NATIVE_LIB_DIR
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|guard| guard.clone())
}

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
    pub native_lib_dir: Option<PathBuf>,
    pub php_info_cache: Arc<Mutex<Option<(Instant, crate::php::PhpInfo)>>>,
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
