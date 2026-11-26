use crate::process::{start_server, stop_server, ServerRow};
use crate::state::AppState;
use crate::util::{db_err, new_id, now_iso, ApiError, ApiResult};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;

#[derive(Serialize)]
pub struct ServerOut {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub engine: String,
    pub port: i64,
    pub bind_lan: bool,
    pub document_root: String,
    pub status: String,
    pub pid: Option<i64>,
    pub started_at: Option<String>,
    pub uptime_seconds: i64,
    pub url_local: String,
    pub url_lan: Option<String>,
    pub error_message: Option<String>,
    pub in_process: bool,
    pub config_text: Option<String>,
    pub proxy_port: Option<i64>,
    pub command: Option<String>,
    pub backend_port: Option<i64>,
    pub warning_message: Option<String>,
    pub last_health_at: Option<String>,
    pub last_health_status: Option<i64>,
    pub last_health_error: Option<String>,
    pub last_health_target: Option<String>,
    pub health_failures: i64,
    pub auto_restart_count: i64,
}

pub fn server_out(row: &sqlx::sqlite::SqliteRow) -> ServerOut {
    let port: i64 = row.get("port");
    let bind_lan: bool = row.get::<i64, _>("bind_lan") != 0;
    let status: String = row.get("status");
    let started_at: Option<String> = row.get("started_at");
    let uptime = if status == "running" || status == "degraded" {
        started_at
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0))
            .unwrap_or(0)
    } else {
        0
    };
    let url_lan = if bind_lan {
        crate::network::lan_ip().map(|ip| format!("http://{}:{}", ip, port))
    } else {
        None
    };
    let stored_warning: Option<String> = row.try_get("warning_message").ok().flatten();
    let warning_message = stored_warning.or_else(|| {
        let project_path: String = row.try_get("project_path").unwrap_or_default();
        if project_path.is_empty() {
            return None;
        }
        let document_root: String = row.get("document_root");
        let doc_root = crate::util::resolve_inside(
            &std::path::PathBuf::from(&project_path),
            &document_root,
            true,
        )
        .ok()?;
        let effective_proxy = row
            .try_get::<Option<i64>, _>("proxy_port")
            .ok()
            .flatten()
            .and_then(|p| u16::try_from(p).ok())
            .or_else(|| {
                row.try_get::<Option<i64>, _>("backend_port")
                    .ok()
                    .flatten()
                    .and_then(|p| u16::try_from(p).ok())
            });
        crate::process::docroot_warning(
            &row.get::<String, _>("engine"),
            &std::path::PathBuf::from(project_path),
            &doc_root,
            effective_proxy,
        )
    });
    ServerOut {
        id: row.get("id"),
        project_id: row.get("project_id"),
        project_name: row.try_get("project_name").unwrap_or_default(),
        engine: row.get("engine"),
        port,
        bind_lan,
        document_root: row.get("document_root"),
        status,
        pid: row.get("pid"),
        started_at,
        uptime_seconds: uptime,
        url_local: format!("http://127.0.0.1:{}", port),
        url_lan,
        error_message: row.get("error_message"),
        in_process: row.get::<i64, _>("in_process") != 0,
        config_text: row.try_get("config_text").ok().flatten(),
        proxy_port: row.try_get("proxy_port").ok().flatten(),
        command: row.try_get("command").ok().flatten(),
        backend_port: row.try_get("backend_port").ok().flatten(),
        warning_message,
        last_health_at: row.try_get("last_health_at").ok().flatten(),
        last_health_status: row.try_get("last_health_status").ok().flatten(),
        last_health_error: row.try_get("last_health_error").ok().flatten(),
        last_health_target: row.try_get("last_health_target").ok().flatten(),
        health_failures: row.try_get("health_failures").unwrap_or(0),
        auto_restart_count: row.try_get("auto_restart_count").unwrap_or(0),
    }
}

const SERVER_SELECT: &str = "SELECT s.*, p.name AS project_name, p.path AS project_path FROM server_instances s JOIN projects p ON p.id = s.project_id";

pub async fn get_server_row(pool: &sqlx::SqlitePool, id: &str) -> ApiResult<sqlx::sqlite::SqliteRow> {
    sqlx::query(&format!("{} WHERE s.id=?1", SERVER_SELECT))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| ApiError::not_found("Server tidak ditemukan"))
}

pub async fn list_servers(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(&format!("{} ORDER BY s.created_at DESC", SERVER_SELECT))
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;
    let out: Vec<Value> = rows.iter().map(|r| serde_json::to_value(server_out(r)).unwrap()).collect();
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct CreateServer {
    pub project_id: String,
    pub engine: String,
    pub port: i64,
    pub bind_lan: Option<bool>,
    pub document_root: Option<String>,
    pub proxy_port: Option<i64>,
    pub command: Option<String>,
}

pub async fn create_server(
    State(state): State<AppState>,
    Json(body): Json<CreateServer>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let project = super::projects::get_project_row(&state.pool, &body.project_id).await?;
    if body.engine == "php" {
        if crate::php::find_php_binary(state.native_lib_dir.as_deref()).is_none() {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "PHP tidak tersedia: libphp.so tidak ditemukan di nativeLibraryDir dan php tidak ada di PATH",
            ));
        }
    } else if body.engine != "static" {
        let runtimes = crate::runtime::runtimes_cached(&state, false).await;
        let available = runtimes.iter().any(|r| r.id == body.engine && r.available);
        if !available {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Runtime {} belum tersedia", body.engine),
            ));
        }
    }
    if let Some(proxy_port) = body.proxy_port {
        if !(1..=65535).contains(&proxy_port) || proxy_port == body.port {
            return Err(ApiError::bad_request("proxy_port tidak valid"));
        }
    }
    if !(1024..=65535).contains(&body.port) {
        return Err(ApiError::bad_request("Port harus antara 1024 dan 65535"));
    }
    let project_path: String = project.get("path");
    let default_docroot: String = project.get("document_root");
    let document_root = body.document_root.unwrap_or(default_docroot);
    // validate docroot inside project
    let doc_root = crate::util::resolve_inside(
        &std::path::PathBuf::from(&project_path),
        &document_root,
        true,
    )
    .map_err(|e| ApiError::bad_request(format!("Document root tidak valid: {}", e.message)))?;
    if body.engine == "apache" && body.proxy_port.is_some() {
        return Err(ApiError::bad_request(
            "Apache httpd WebForge tidak mendukung reverse proxy: mod_proxy tidak dikompilasi. Gunakan Caddy atau Nginx untuk reverse proxy",
        ));
    }
    if body.engine == "apache"
        && crate::process::is_php_project(&std::path::PathBuf::from(&project_path), &doc_root)
    {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Apache+PHP tidak tersedia di WebForge: index.php tidak akan dieksekusi oleh Apache. Gunakan engine PHP, Nginx, atau Caddy",
        ));
    }
    if matches!(body.engine.as_str(), "nginx" | "caddy")
        && body.proxy_port.is_none()
        && crate::process::is_php_project(&std::path::PathBuf::from(&project_path), &doc_root)
        && crate::php::find_php_binary(state.native_lib_dir.as_deref()).is_none()
    {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Mode komposit PHP membutuhkan runtime PHP untuk backend php -S internal, tetapi PHP tidak tersedia",
        ));
    }
    let id = new_id();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO server_instances (id, project_id, engine, port, bind_lan, document_root, status, proxy_port, command, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,'stopped',?7,?8,?9,?9)",
    )
    .bind(&id)
    .bind(&body.project_id)
    .bind(&body.engine)
    .bind(body.port)
    .bind(if body.bind_lan.unwrap_or(false) { 1 } else { 0 })
    .bind(&document_root)
    .bind(body.proxy_port)
    .bind(&body.command)
    .bind(&now)
    .execute(&state.pool)
    .await
    .map_err(db_err)?;
    let row = get_server_row(&state.pool, &id).await?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(server_out(&row)).unwrap())))
}

pub async fn get_server(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = get_server_row(&state.pool, &id).await?;
    Ok(Json(serde_json::to_value(server_out(&row)).unwrap()))
}

pub async fn delete_server(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = get_server_row(&state.pool, &id).await?;
    let status: String = row.get("status");
    if matches!(status.as_str(), "running" | "degraded" | "starting" | "stopping") {
        return Err(ApiError::conflict("Server harus dihentikan sebelum dihapus"));
    }
    sqlx::query("DELETE FROM server_instances WHERE id=?1")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    Ok(Json(json!({"ok": true})))
}

fn to_server_row(row: &sqlx::sqlite::SqliteRow) -> ServerRow {
    ServerRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        engine: row.get("engine"),
        port: row.get("port"),
        bind_lan: row.get::<i64, _>("bind_lan") != 0,
        document_root: row.get("document_root"),
        status: row.get("status"),
        config_text: row.try_get("config_text").ok().flatten(),
        proxy_port: row.try_get("proxy_port").ok().flatten(),
        command: row.try_get("command").ok().flatten(),
        backend_port: row.try_get("backend_port").ok().flatten(),
        health_failures: row.try_get("health_failures").unwrap_or(0),
        auto_restart_count: row.try_get("auto_restart_count").unwrap_or(0),
    }
}

pub async fn start(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = get_server_row(&state.pool, &id).await?;
    let status: String = row.get("status");
    if status == "running" {
        return Ok(Json(serde_json::to_value(server_out(&row)).unwrap()));
    }
    let project = super::projects::get_project_row(&state.pool, &row.get::<String, _>("project_id")).await?;
    let project_path: String = project.get("path");
    let server = to_server_row(&row);
    // start_server records honest error status internally; return instance either way
    let _ = start_server(&state, &server, &project_path).await;
    let row = get_server_row(&state.pool, &id).await?;
    Ok(Json(serde_json::to_value(server_out(&row)).unwrap()))
}

pub async fn stop(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let _ = get_server_row(&state.pool, &id).await?;
    stop_server(&state, &id).await;
    let row = get_server_row(&state.pool, &id).await?;
    Ok(Json(serde_json::to_value(server_out(&row)).unwrap()))
}

pub async fn restart(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = get_server_row(&state.pool, &id).await?;
    let status: String = row.get("status");
    if matches!(status.as_str(), "running" | "degraded" | "starting") {
        stop_server(&state, &id).await;
    }
    let row = get_server_row(&state.pool, &id).await?;
    let project = super::projects::get_project_row(&state.pool, &row.get::<String, _>("project_id")).await?;
    let project_path: String = project.get("path");
    let server = to_server_row(&row);
    let _ = start_server(&state, &server, &project_path).await;
    let row = get_server_row(&state.pool, &id).await?;
    Ok(Json(serde_json::to_value(server_out(&row)).unwrap()))
}

pub async fn test_lan(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = get_server_row(&state.pool, &id).await?;
    let server = to_server_row(&row);
    let result = crate::process::test_lan_access(&state, &server).await;
    Ok(Json(serde_json::to_value(result).unwrap()))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    pub limit: Option<i64>,
}

pub async fn server_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Json<Value>> {
    let _ = get_server_row(&state.pool, &id).await?;
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let rows = sqlx::query(
        "SELECT id, severity AS level, message, created_at FROM application_logs WHERE server_id=?1 ORDER BY created_at DESC LIMIT ?2",
    )
    .bind(&id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(db_err)?;
    let mut out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "level": r.get::<String, _>("level"),
                "message": r.get::<String, _>("message"),
                "created_at": r.get::<String, _>("created_at"),
            })
        })
        .collect();
    out.reverse();
    Ok(Json(json!(out)))
}


#[derive(Serialize)]
pub struct ServerConfigOut {
    pub engine: String,
    pub config_text: String,
    pub generated_config: String,
    pub config_path: String,
    pub editable: bool,
}

fn config_out(state: &AppState, server: &ServerRow, config_text: String, generated: String) -> ServerConfigOut {
    let home = crate::webservers::server_home(&state.config.data_dir, &server.id, &server.engine);
    let filename = crate::webservers::config_filename(&server.engine).unwrap_or("config");
    ServerConfigOut {
        engine: server.engine.clone(),
        config_text,
        generated_config: generated,
        config_path: home.join(filename).to_string_lossy().to_string(),
        editable: true,
    }
}

#[derive(Deserialize)]
pub struct PutConfigBody {
    pub config_text: String,
}

#[derive(Serialize)]
pub struct ValidateConfigOut {
    pub valid: bool,
    pub output: String,
}

async fn server_and_project_path(
    state: &AppState,
    server_id: &str,
) -> ApiResult<(ServerRow, String)> {
    let row = get_server_row(&state.pool, server_id).await?;
    let server = to_server_row(&row);
    let project = super::projects::get_project_row(&state.pool, &server.project_id).await?;
    Ok((server, project.get::<String, _>("path")))
}

async fn generated_config(state: &AppState, server: &ServerRow, project_path: &str) -> ApiResult<String> {
    let doc_root = crate::util::resolve_inside(
        &std::path::PathBuf::from(project_path),
        &server.document_root,
        true,
    )
    .map_err(|e| ApiError::bad_request(e.message))?;
    let proxy_port = crate::process::effective_proxy_port(
        state,
        server,
        &std::path::PathBuf::from(project_path),
        &doc_root,
        true,
    )
    .await;
    let input = crate::webservers::WebServerInput {
        host: if server.bind_lan { "0.0.0.0".to_string() } else { "127.0.0.1".to_string() },
        port: server.port as u16,
        doc_root,
        project_path: std::path::PathBuf::from(project_path),
        data_dir: state.config.data_dir.clone(),
        server_id: server.id.clone(),
        proxy_port,
        custom_config: None,
    };
    match server.engine.as_str() {
        "caddy" => Ok(crate::webservers::generate_caddyfile(&input)),
        "nginx" => Ok(crate::webservers::generate_nginx_config(&input)),
        "apache" => crate::webservers::generate_httpd_config(&input).map_err(ApiError::bad_request),
        other => Err(ApiError::bad_request(format!(
            "Engine {} tidak memiliki editor konfigurasi",
            other
        ))),
    }
}

pub async fn get_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let (server, project_path) = server_and_project_path(&state, &id).await?;
    let generated = generated_config(&state, &server, &project_path).await?;
    let out = config_out(
        &state,
        &server,
        server.config_text.clone().unwrap_or_else(|| generated.clone()),
        generated,
    );
    Ok(Json(serde_json::to_value(out).unwrap()))
}

pub async fn put_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PutConfigBody>,
) -> ApiResult<Json<Value>> {
    let (server, project_path) = server_and_project_path(&state, &id).await?;
    let generated = generated_config(&state, &server, &project_path).await?;
    sqlx::query("UPDATE server_instances SET config_text=?1, updated_at=?2 WHERE id=?3")
        .bind(&body.config_text)
        .bind(now_iso())
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    let out = config_out(&state, &server, body.config_text, generated);
    Ok(Json(serde_json::to_value(out).unwrap()))
}

pub async fn validate_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PutConfigBody>,
) -> ApiResult<Json<Value>> {
    let (server, project_path) = server_and_project_path(&state, &id).await?;
    let (valid, output) = crate::process::validate_webserver_config(
        &state,
        &server,
        &project_path,
        Some(&body.config_text),
    )
    .await
    .map_err(ApiError::bad_request)?;
    let out = ValidateConfigOut { valid, output };
    Ok(Json(serde_json::to_value(out).unwrap()))
}

pub async fn reload_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let (server, project_path) = server_and_project_path(&state, &id).await?;
    crate::process::reload_server(&state, &server, &project_path)
        .await
        .map_err(ApiError::bad_request)?;
    let row = get_server_row(&state.pool, &id).await?;
    Ok(Json(serde_json::to_value(server_out(&row)).unwrap()))
}
