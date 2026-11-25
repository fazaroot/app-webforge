use crate::state::AppState;
use crate::util::{db_err, is_inside, new_id, now_iso, ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Column, Row, TypeInfo};
use std::path::PathBuf;
use std::str::FromStr;

async fn db_row(state: &AppState, id: &str) -> ApiResult<sqlx::sqlite::SqliteRow> {
    sqlx::query("SELECT * FROM database_instances WHERE id=?1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| ApiError::not_found("Database tidak ditemukan"))
}

async fn open_db(path: &str) -> ApiResult<sqlx::SqlitePool> {
    let url = format!("sqlite://{}?mode=rwc", path);
    let opts = SqliteConnectOptions::from_str(&url)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .create_if_missing(true)
        .foreign_keys(true);
    sqlx::SqlitePool::connect_with(opts)
        .await
        .map_err(|e| ApiError::internal(format!("Gagal membuka database: {}", e)))
}

fn db_out(row: &sqlx::sqlite::SqliteRow) -> Value {
    let path: String = row.get("path");
    let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
    json!({
        "id": row.get::<String, _>("id"),
        "name": row.get::<String, _>("name"),
        "path": path,
        "size_bytes": size,
        "created_at": row.get::<String, _>("created_at"),
    })
}

pub async fn list_databases(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query("SELECT * FROM database_instances ORDER BY created_at DESC")
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;
    Ok(Json(json!(rows.iter().map(db_out).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub struct CreateDb {
    pub name: String,
}

pub async fn create_database(State(state): State<AppState>, Json(b): Json<CreateDb>) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = b.name.trim().to_string();
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request("Nama database hanya boleh huruf, angka, '-' dan '_'"));
    }
    let path = state.config.databases_dir().join(format!("{}.db", name));
    if path.exists() {
        return Err(ApiError::conflict("Database dengan nama itu sudah ada"));
    }
    let pool = open_db(&path.to_string_lossy()).await?;
    // Ensure file is a valid sqlite db
    sqlx::query("PRAGMA user_version").execute(&pool).await.map_err(db_err)?;
    pool.close().await;
    let id = new_id();
    let now = now_iso();
    let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
    sqlx::query("INSERT INTO database_instances (id, name, path, size_bytes, created_at) VALUES (?1,?2,?3,?4,?5)")
        .bind(&id)
        .bind(&name)
        .bind(path.to_string_lossy().to_string())
        .bind(size)
        .bind(&now)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    let row = db_row(&state, &id).await?;
    Ok((StatusCode::CREATED, Json(db_out(&row))))
}

pub async fn delete_database(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let path: String = row.get("path");
    sqlx::query("DELETE FROM database_instances WHERE id=?1")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    let p = PathBuf::from(&path);
    if is_inside(&state.config.databases_dir(), &p) && p.exists() {
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{}-wal", path));
        let _ = std::fs::remove_file(format!("{}-shm", path));
    }
    Ok(Json(json!({"ok": true})))
}

pub async fn list_tables(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let path: String = row.get("path");
    let pool = open_db(&path).await?;
    let tables = sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .fetch_all(&pool)
        .await
        .map_err(db_err)?;
    let mut out = Vec::new();
    for t in &tables {
        let tname: String = t.get("name");
        let cols = sqlx::query(&format!("PRAGMA table_info(\"{}\")", tname.replace('"', "\"\"")))
            .fetch_all(&pool)
            .await
            .map_err(db_err)?;
        let columns: Vec<Value> = cols
            .iter()
            .map(|c| {
                json!({
                    "name": c.get::<String, _>("name"),
                    "type": c.get::<String, _>("type"),
                    "pk": c.get::<i64, _>("pk") > 0,
                })
            })
            .collect();
        out.push(json!({"name": tname, "columns": columns}));
    }
    pool.close().await;
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct QueryBody {
    pub sql: String,
}

fn value_of(row: &sqlx::sqlite::SqliteRow, idx: usize) -> Value {
    let col = &row.columns()[idx];
    let type_name = col.type_info().name().to_uppercase();
    match type_name.as_str() {
        "INTEGER" | "INT" | "BIGINT" => row
            .try_get::<Option<i64>, _>(idx)
            .ok()
            .flatten()
            .map(|v| json!(v))
            .unwrap_or(Value::Null),
        "REAL" | "FLOAT" | "DOUBLE" => row
            .try_get::<Option<f64>, _>(idx)
            .ok()
            .flatten()
            .map(|v| json!(v))
            .unwrap_or(Value::Null),
        "BLOB" => row
            .try_get::<Option<Vec<u8>>, _>(idx)
            .ok()
            .flatten()
            .map(|v| json!(format!("<blob {} bytes>", v.len())))
            .unwrap_or(Value::Null),
        _ => {
            if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
                return v.map(|s| json!(s)).unwrap_or(Value::Null);
            }
            if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
                return v.map(|n| json!(n)).unwrap_or(Value::Null);
            }
            if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
                return v.map(|n| json!(n)).unwrap_or(Value::Null);
            }
            Value::Null
        }
    }
}

pub async fn run_query(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<QueryBody>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let path: String = row.get("path");
    let sql = b.sql.trim().to_string();
    if sql.is_empty() {
        return Err(ApiError::bad_request("SQL wajib diisi"));
    }
    let pool = open_db(&path).await?;
    let upper = sql.to_uppercase();
    let is_select = upper.starts_with("SELECT") || upper.starts_with("PRAGMA") || upper.starts_with("WITH") || upper.starts_with("EXPLAIN");
    let result = if is_select {
        match sqlx::query(&sql).fetch_all(&pool).await {
            Ok(rows) => {
                let columns: Vec<String> = rows
                    .first()
                    .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default();
                let out_rows: Vec<Value> = rows
                    .iter()
                    .map(|r| json!(r.columns().iter().enumerate().map(|(i, _)| value_of(r, i)).collect::<Vec<_>>()))
                    .collect();
                Ok(json!({"columns": columns, "rows": out_rows, "rows_affected": 0}))
            }
            Err(e) => Err(ApiError::bad_request(format!("Error SQL: {}", e))),
        }
    } else {
        match sqlx::query(&sql).execute(&pool).await {
            Ok(res) => Ok(json!({"columns": [], "rows": [], "rows_affected": res.rows_affected()})),
            Err(e) => Err(ApiError::bad_request(format!("Error SQL: {}", e))),
        }
    };
    pool.close().await;
    result.map(Json)
}

pub async fn backup_database(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let path: String = row.get("path");
    let name: String = row.get("name");
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let backup_path = state.config.backups_dir().join(format!("{}-{}.db", name, stamp));
    std::fs::copy(&path, &backup_path).map_err(|e| ApiError::internal(format!("Gagal backup: {}", e)))?;
    let now = now_iso();
    sqlx::query("INSERT INTO backup_history (id, database_id, backup_path, created_at) VALUES (?1,?2,?3,?4)")
        .bind(new_id())
        .bind(&id)
        .bind(backup_path.to_string_lossy().to_string())
        .bind(&now)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    Ok(Json(json!({"backup_path": backup_path.to_string_lossy().to_string(), "created_at": now})))
}

#[derive(Deserialize)]
pub struct RestoreBody {
    pub backup_path: String,
}

pub async fn restore_database(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<RestoreBody>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let path: String = row.get("path");
    let backup = PathBuf::from(&b.backup_path);
    let canon = backup
        .canonicalize()
        .map_err(|_| ApiError::bad_request("File backup tidak ditemukan"))?;
    if !is_inside(&state.config.backups_dir(), &canon) {
        return Err(ApiError::bad_request("Path backup harus berada di folder backups"));
    }
    std::fs::copy(&canon, &path).map_err(|e| ApiError::internal(format!("Gagal restore: {}", e)))?;
    let _ = std::fs::remove_file(format!("{}-wal", path));
    let _ = std::fs::remove_file(format!("{}-shm", path));
    Ok(Json(json!({"ok": true})))
}
