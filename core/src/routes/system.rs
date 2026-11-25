use crate::state::AppState;
use crate::util::{db_err, now_iso, ApiError, ApiResult};
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

pub async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.uptime_seconds(),
    }))
}

pub async fn system_info(State(state): State<AppState>) -> Json<Value> {
    let runtimes = crate::runtime::sync_runtime_table(&state.pool).await;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "data_dir": state.config.data_dir.to_string_lossy().to_string(),
        "workspace_dir": state.config.workspace_dir.to_string_lossy().to_string(),
        "management_port": state.config.port,
        "started_at": crate::util::iso_of(state.started_at),
        "runtimes": serde_json::to_value(&runtimes).unwrap_or(json!([])),
    }))
}

pub async fn runtimes(State(state): State<AppState>) -> Json<Value> {
    let runtimes = crate::runtime::sync_runtime_table(&state.pool).await;
    Json(serde_json::to_value(&runtimes).unwrap_or(json!([])))
}

pub async fn diagnostics(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let mut checks: Vec<Value> = Vec::new();
    let mut check = |name: &str, status: &str, detail: String| {
        checks.push(json!({"name": name, "status": status, "detail": detail}));
    };
    // data dir writable
    let probe = state.config.data_dir.join(".wf-write-probe");
    match std::fs::write(&probe, "ok") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            check("Data directory", "ok", format!("{} dapat ditulis", state.config.data_dir.display()));
        }
        Err(e) => check("Data directory", "error", format!("Tidak dapat ditulis: {}", e)),
    }
    if state.config.workspace_dir.exists() {
        check("Workspace", "ok", state.config.workspace_dir.to_string_lossy().to_string());
    } else {
        check("Workspace", "error", "Folder workspace tidak ditemukan".to_string());
    }
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => check("Database", "ok", "SQLite merespons".to_string()),
        Err(e) => check("Database", "error", format!("SQLite error: {}", e)),
    }
    check("Management API", "ok", format!("Bind 127.0.0.1:{}", state.config.port));
    let runtimes = crate::runtime::sync_runtime_table(&state.pool).await;
    let unavailable: Vec<String> = runtimes.iter().filter(|r| !r.available).map(|r| r.name.clone()).collect();
    if unavailable.is_empty() {
        check("Runtimes", "ok", "Semua runtime tersedia".to_string());
    } else {
        check("Runtimes", "warn", format!("Belum tersedia: {}", unavailable.join(", ")));
    }
    let storage = crate::monitoring::storage_info(&state.config.workspace_dir);
    let (s_total, s_used) = storage.unwrap_or((0, 0));
    if storage.is_some() && s_total > 0 {
        let pct = s_used as f64 / s_total as f64 * 100.0;
        if pct > 90.0 {
            check("Storage", "warn", format!("Penggunaan storage {:.1}%", pct));
        } else {
            check("Storage", "ok", format!("Penggunaan storage {:.1}%", pct));
        }
    } else {
        check("Storage", "warn", "Info storage tidak tersedia".to_string());
    }
    let port_rows = sqlx::query("SELECT port, server_id FROM port_allocations ORDER BY port")
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;
    let ports: Vec<Value> = port_rows
        .iter()
        .map(|r| json!({"port": r.get::<i64, _>("port"), "server_id": r.get::<String, _>("server_id")}))
        .collect();
    Ok(Json(json!({
        "generated_at": now_iso(),
        "checks": checks,
        "runtimes": serde_json::to_value(&runtimes).unwrap_or(json!([])),
        "ports": ports,
        "storage": {"total_bytes": s_total, "used_bytes": s_used},
    })))
}

pub async fn monitoring_current(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let m = crate::monitoring::collect_metrics(&state.config.workspace_dir).await;
    let rows = sqlx::query(
        "SELECT s.id, s.pid, s.started_at, p.name FROM server_instances s JOIN projects p ON p.id=s.project_id WHERE s.status='running'",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(db_err)?;
    let processes: Vec<Value> = rows
        .iter()
        .map(|r| {
            let started: Option<String> = r.get("started_at");
            let uptime = started
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0))
                .unwrap_or(0);
            json!({
                "pid": r.get::<Option<i64>, _>("pid"),
                "name": r.get::<String, _>("name"),
                "server_id": r.get::<String, _>("id"),
                "uptime_seconds": uptime,
            })
        })
        .collect();
    Ok(Json(json!({
        "cpu_percent": m.cpu_percent,
        "mem_total_bytes": m.mem_total_bytes,
        "mem_used_bytes": m.mem_used_bytes,
        "storage_total_bytes": m.storage_total_bytes,
        "storage_used_bytes": m.storage_used_bytes,
        "processes": processes,
        "ports": serde_json::to_value(&m.ports).unwrap_or(json!([])),
        "unavailable_reason": m.unavailable_reason,
    })))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

pub async fn monitoring_history(State(state): State<AppState>, Query(q): Query<HistoryQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(60).clamp(1, 500);
    let rows = sqlx::query(
        "SELECT recorded_at, cpu_percent, mem_used_bytes, storage_used_bytes FROM monitoring_metrics ORDER BY recorded_at DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(db_err)?;
    let mut out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "recorded_at": r.get::<String, _>("recorded_at"),
                "cpu_percent": r.get::<Option<f64>, _>("cpu_percent"),
                "mem_used_bytes": r.get::<Option<i64>, _>("mem_used_bytes"),
                "storage_used_bytes": r.get::<Option<i64>, _>("storage_used_bytes"),
            })
        })
        .collect();
    out.reverse();
    Ok(Json(json!(out)))
}

pub async fn network(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "lan_ip": crate::network::lan_ip(),
        "interfaces": serde_json::to_value(crate::network::interfaces()).unwrap_or(json!([])),
        "management_url": format!("http://127.0.0.1:{}", state.config.port),
    }))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    pub severity: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
}

pub async fn logs(State(state): State<AppState>, Query(q): Query<LogsQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let rows = sqlx::query(
        "SELECT id, category, severity, message, created_at FROM application_logs
         WHERE (?1 IS NULL OR severity=?1) AND (?2 IS NULL OR message LIKE '%' || ?2 || '%')
         ORDER BY created_at DESC LIMIT ?3",
    )
    .bind(&q.severity)
    .bind(&q.search)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(db_err)?;
    let mut out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "category": r.get::<String, _>("category"),
                "severity": r.get::<String, _>("severity"),
                "message": r.get::<String, _>("message"),
                "created_at": r.get::<String, _>("created_at"),
            })
        })
        .collect();
    out.reverse();
    Ok(Json(json!(out)))
}

pub async fn logs_export(State(state): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query("SELECT category, severity, message, created_at FROM application_logs ORDER BY created_at ASC LIMIT 5000")
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;
    let mut text = String::new();
    for r in &rows {
        text.push_str(&format!(
            "[{}] [{}] [{}] {}\n",
            r.get::<String, _>("created_at"),
            r.get::<String, _>("severity"),
            r.get::<String, _>("category"),
            r.get::<String, _>("message")
        ));
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"webforge-logs.txt\"".to_string()),
        ],
        text,
    ))
}

#[derive(Deserialize)]
pub struct GitQuery {
    pub project_id: String,
}

pub async fn git_status(State(state): State<AppState>, Query(q): Query<GitQuery>) -> ApiResult<Json<Value>> {
    let project = super::projects::get_project_row(&state.pool, &q.project_id).await?;
    let path: String = project.get("path");
    let git = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).map(|d| d.join("git")).find(|c| c.is_file())
    });
    let Some(git_path) = git else {
        return Ok(Json(json!({
            "available": false,
            "reason": "git tidak ditemukan di PATH",
            "branch": null,
            "clean": false,
            "entries": [],
        })));
    };
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new(&git_path)
            .arg("-C")
            .arg(&path)
            .args(["status", "--porcelain=v1", "-b"])
            .output(),
    )
    .await;
    match out {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !output.status.success() {
                return Ok(Json(json!({
                    "available": true,
                    "reason": if stderr.contains("not a git repository") { "Folder proyek bukan repository git".to_string() } else { stderr.trim().to_string() },
                    "branch": null,
                    "clean": false,
                    "entries": [],
                })));
            }
            let mut branch: Option<String> = None;
            let mut entries: Vec<String> = Vec::new();
            for line in stdout.lines() {
                if let Some(rest) = line.strip_prefix("## ") {
                    branch = Some(rest.split("...").next().unwrap_or(rest).trim().to_string());
                } else if !line.is_empty() {
                    entries.push(line.to_string());
                }
            }
            Ok(Json(json!({
                "available": true,
                "reason": null,
                "branch": branch,
                "clean": entries.is_empty(),
                "entries": entries,
            })))
        }
        Ok(Err(e)) => Err(ApiError::internal(format!("Gagal menjalankan git: {}", e))),
        Err(_) => Err(ApiError::internal("git status timeout")),
    }
}

pub async fn get_settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query("SELECT key, value FROM user_settings")
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;
    let mut obj = serde_json::Map::new();
    obj.insert("theme".to_string(), json!("system"));
    obj.insert("lan_default".to_string(), json!(false));
    for r in &rows {
        let key: String = r.get("key");
        let raw: String = r.get("value");
        let value: Value = serde_json::from_str(&raw).unwrap_or(json!(raw));
        obj.insert(key, value);
    }
    Ok(Json(Value::Object(obj)))
}

pub async fn put_settings(State(state): State<AppState>, Json(body): Json<Value>) -> ApiResult<Json<Value>> {
    let obj = body.as_object().ok_or_else(|| ApiError::bad_request("Body harus objek JSON"))?;
    let now = now_iso();
    for (k, v) in obj {
        if k.is_empty() || k.len() > 100 {
            return Err(ApiError::bad_request("Key setting tidak valid"));
        }
        sqlx::query(
            "INSERT INTO user_settings (key, value, updated_at) VALUES (?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(k)
        .bind(v.to_string())
        .bind(&now)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    }
    get_settings(State(state)).await
}

pub async fn security_status(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let lan_servers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM server_instances WHERE bind_lan=1 AND status='running'")
        .fetch_one(&state.pool)
        .await
        .map_err(db_err)?;
    let mut warnings: Vec<String> = Vec::new();
    if lan_servers > 0 {
        warnings.push(format!("{} server dapat diakses dari LAN", lan_servers));
    }
    if std::env::var("WEBFORGE_API_TOKEN").is_ok() {
        warnings.push("Token API diambil dari environment variable WEBFORGE_API_TOKEN".to_string());
    }
    Ok(Json(json!({
        "api_token_set": !state.config.token.is_empty(),
        "management_bind": "127.0.0.1",
        "lan_servers": lan_servers,
        "warnings": warnings,
    })))
}
