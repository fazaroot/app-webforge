pub mod config;
pub mod db;
pub mod db_runtime;
pub mod framework;
pub mod monitoring;
pub mod network;
pub mod node_runtime;
pub mod php;
pub mod process;
pub mod python_runtime;
pub mod routes;
pub mod runtime;
pub mod secrets;
pub mod selftest;
pub mod sftp;
pub mod state;
pub mod templates;
pub mod util;
pub mod webservers;

#[cfg(target_os = "android")]
pub mod jni_bridge;

use config::Config;
use state::AppState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::services::{ServeDir, ServeFile};

/// Static file app: ServeDir with index.html fallback (SPA-style).
pub fn static_app(root: PathBuf) -> axum::Router {
    let index = root.join("index.html");
    let serve_dir = ServeDir::new(&root)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index));
    axum::Router::new().fallback_service(serve_dir)
}

pub async fn run_static_server(
    root: PathBuf,
    host: String,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !root.is_dir() {
        return Err(format!("Document root bukan folder: {}", root.display()).into());
    }
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    tracing::info!("Static server {} di {}:{}", root.display(), host, port);
    println!("webforge static server: {} di {}:{}", root.display(), host, port);
    axum::serve(listener, static_app(root)).await?;
    Ok(())
}

async fn build_state(config: Config) -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    state::set_data_dir(config.data_dir.clone());
    let pool = db::init_pool(&config).await?;
    let state = AppState {
        pool,
        config,
        started_at: chrono::Utc::now(),
        started_instant: std::time::Instant::now(),
        processes: Arc::new(Mutex::new(HashMap::new())),
        native_lib_dir: state::native_lib_dir(),
        php_info_cache: Arc::new(Mutex::new(None)),
        runtime_cache: Arc::new(runtime::RuntimeCache::new()),
        process_stderr: Arc::new(Mutex::new(HashMap::new())),
        log_tail_tasks: Arc::new(Mutex::new(HashMap::new())),
        log_tail_offsets: Arc::new(Mutex::new(HashMap::new())),
        health_restarting: Arc::new(Mutex::new(HashSet::new())),
        sftp: Arc::new(sftp::SftpHandle::new()),
    };
    // Probe runtime sekali saat core start; hasilnya masuk cache TTL
    // 60 detik dan tabel runtime_versions.
    let _ = runtime::sync_runtime_table(&state, false).await;
    monitoring::spawn_sampler(state.pool.clone(), state.config.workspace_dir.clone());
    process::spawn_health_supervisor(state.clone());
    Ok(state)
}

pub async fn run_management(config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = tx.send(());
    });
    run_with_shutdown(config, rx).await
}

pub async fn run_with_shutdown(
    config: Config,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port = config.port;
    let state = build_state(config).await?;
    let app = routes::router(state.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    tracing::info!("WebForge management API di 127.0.0.1:{}", port);
    println!("webforge-core management API: http://127.0.0.1:{}", port);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await?;
    // Stop all child servers on shutdown
    let ids: Vec<String> = state.processes.lock().await.keys().cloned().collect();
    for id in ids {
        process::stop_server(&state, &id).await;
    }
    state.sftp.shutdown(&state.pool).await;
    Ok(())
}
