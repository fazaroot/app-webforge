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
}

pub fn server_out(row: &sqlx::sqlite::SqliteRow) -> ServerOut {
    let port: i64 = row.get("port");
    let bind_lan: bool = row.get::<i64, _>("bind_lan") != 0;
    let status: String = row.get("status");
    let started_at: Option<String> = row.get("started_at");
    let uptime = if status == "running" {
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
    }
}

const SERVER_SELECT: &str = "SELECT s.*, p.name AS project_name FROM server_instances s JOIN projects p ON p.id = s.project_id";

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
        let runtimes = crate::runtime::detect_runtimes().await;
        let available = runtimes.iter().any(|r| r.id == body.engine && r.available);
        if !available {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Runtime {} belum tersedia", body.engine),
            ));
        }
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Runtime {} terdeteksi tetapi belum didukung sebagai engine server", body.engine),
        ));
    }
    if !(1024..=65535).contains(&body.port) {
        return Err(ApiError::bad_request("Port harus antara 1024 dan 65535"));
    }
    let project_path: String = project.get("path");
    let default_docroot: String = project.get("document_root");
    let document_root = body.document_root.unwrap_or(default_docroot);
    // validate docroot inside project
    crate::util::resolve_inside(&std::path::PathBuf::from(&project_path), &document_root, true)
        .map_err(|e| ApiError::bad_request(format!("Document root tidak valid: {}", e.message)))?;
    let id = new_id();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO server_instances (id, project_id, engine, port, bind_lan, document_root, status, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,'stopped',?7,?7)",
    )
    .bind(&id)
    .bind(&body.project_id)
    .bind(&body.engine)
    .bind(body.port)
    .bind(if body.bind_lan.unwrap_or(false) { 1 } else { 0 })
    .bind(&document_root)
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
    if status == "running" || status == "starting" {
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
    if status == "running" || status == "starting" {
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
