pub mod databases;
pub mod files;
pub mod projects;
pub mod servers;
pub mod system;
pub mod terminal;

use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::cors::{AllowOrigin, CorsLayer};

async fn auth_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let is_health = path == "/api/health" && req.method() == Method::GET;
    // WebSocket terminal validates its token via ?token= inside the handler.
    let is_ws = path == "/ws/terminal";
    if is_health || is_ws {
        return next.run(req).await;
    }
    let token = &state.config.token;
    let header_key = req
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if header_key.as_deref() == Some(token.as_str()) || bearer.as_deref() == Some(token.as_str()) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response()
    }
}

pub fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _req| {
            let s = origin.to_str().unwrap_or("");
            s == "capacitor://localhost"
                || s == "https://localhost"
                || s.starts_with("http://localhost:")
                || s == "http://localhost"
        }))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, "x-api-key".parse().unwrap()]);

    let api = Router::new()
        .route("/api/health", get(system::health))
        .route("/api/system/info", get(system::system_info))
        .route("/api/system/diagnostics", get(system::diagnostics))
        .route("/api/runtimes", get(system::runtimes))
        .route("/api/projects", get(projects::list_projects).post(projects::create_project))
        .route("/api/projects/register", post(projects::register_project))
        .route("/api/projects/:id", get(projects::get_project).delete(projects::delete_project))
        .route("/api/projects/:id/scan", post(projects::scan_project))
        .route("/api/servers", get(servers::list_servers).post(servers::create_server))
        .route("/api/servers/:id", get(servers::get_server).delete(servers::delete_server))
        .route("/api/servers/:id/start", post(servers::start))
        .route("/api/servers/:id/stop", post(servers::stop))
        .route("/api/servers/:id/restart", post(servers::restart))
        .route("/api/servers/:id/logs", get(servers::server_logs))
        .route("/api/files/list", get(files::list_files))
        .route("/api/files/read", get(files::read_file))
        .route("/api/files/write", post(files::write_file))
        .route("/api/files/mkdir", post(files::mkdir))
        .route("/api/files/rename", post(files::rename_file))
        .route("/api/files/delete", post(files::delete_file))
        .route("/api/files/copy", post(files::copy_file))
        .route("/api/files/move", post(files::move_file))
        .route("/api/files/search", get(files::search_files))
        .route("/api/files/dir-size", get(files::dir_size))
        .route("/api/databases", get(databases::list_databases).post(databases::create_database))
        .route("/api/databases/:id", delete(databases::delete_database))
        .route("/api/databases/:id/tables", get(databases::list_tables))
        .route("/api/databases/:id/query", post(databases::run_query))
        .route("/api/databases/:id/backup", post(databases::backup_database))
        .route("/api/databases/:id/restore", post(databases::restore_database))
        .route("/api/monitoring/current", get(system::monitoring_current))
        .route("/api/monitoring/history", get(system::monitoring_history))
        .route("/api/terminal/execute", post(terminal::execute))
        .route("/api/terminal/history", get(terminal::history))
        .route("/api/network", get(system::network))
        .route("/api/logs", get(system::logs))
        .route("/api/logs/export", get(system::logs_export))
        .route("/api/tools/git/status", get(system::git_status))
        .route("/api/settings", get(system::get_settings).put(system::put_settings))
        .route("/api/security/status", get(system::security_status))
        .route("/ws/terminal", get(terminal::ws_terminal))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware))
        .layer(cors);

    api.with_state(state)
}
