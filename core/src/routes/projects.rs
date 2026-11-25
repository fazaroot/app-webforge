use crate::state::AppState;
use crate::util::{db_err, is_inside, new_id, now_iso, slugify, ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct ProjectOut {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub path: String,
    pub document_root: String,
    pub created_at: String,
    pub updated_at: String,
    pub framework: Option<Value>,
}

pub async fn framework_json(pool: &sqlx::SqlitePool, project_id: &str) -> Option<Value> {
    let row = sqlx::query(
        "SELECT framework, language, version, recommended_runtime, recommended_document_root, readiness, notes, detected_files
         FROM project_frameworks WHERE project_id=?1 ORDER BY detected_at DESC LIMIT 1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    let notes: Value = serde_json::from_str(&row.get::<String, _>("notes")).unwrap_or(json!([]));
    let detected: Value = serde_json::from_str(&row.get::<String, _>("detected_files")).unwrap_or(json!([]));
    Some(json!({
        "framework": row.get::<String, _>("framework"),
        "language": row.get::<String, _>("language"),
        "version": row.get::<Option<String>, _>("version"),
        "recommended_runtime": row.get::<String, _>("recommended_runtime"),
        "recommended_document_root": row.get::<String, _>("recommended_document_root"),
        "readiness": row.get::<String, _>("readiness"),
        "notes": notes,
        "detected_files": detected,
    }))
}

pub async fn project_out(pool: &sqlx::SqlitePool, row: &sqlx::sqlite::SqliteRow) -> ProjectOut {
    let id: String = row.get("id");
    let framework = framework_json(pool, &id).await;
    ProjectOut {
        id,
        name: row.get("name"),
        slug: row.get("slug"),
        path: row.get("path"),
        document_root: row.get("document_root"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        framework,
    }
}

pub async fn get_project_row(pool: &sqlx::SqlitePool, id: &str) -> ApiResult<sqlx::sqlite::SqliteRow> {
    sqlx::query("SELECT * FROM projects WHERE id=?1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| ApiError::not_found("Proyek tidak ditemukan"))
}

pub async fn run_scan(state: &AppState, project_id: &str, project_path: &str) -> ApiResult<Value> {
    let available = crate::runtime::available_runtime_ids(state).await;
    let info = crate::framework::detect(&PathBuf::from(project_path), &available);
    let now = now_iso();
    sqlx::query("DELETE FROM project_frameworks WHERE project_id=?1")
        .bind(project_id)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    sqlx::query(
        "INSERT INTO project_frameworks (id, project_id, framework, language, version, recommended_runtime, recommended_document_root, readiness, notes, detected_files, detected_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
    )
    .bind(new_id())
    .bind(project_id)
    .bind(&info.framework)
    .bind(&info.language)
    .bind(&info.version)
    .bind(&info.recommended_runtime)
    .bind(&info.recommended_document_root)
    .bind(&info.readiness)
    .bind(serde_json::to_string(&info.notes).unwrap_or("[]".into()))
    .bind(serde_json::to_string(&info.detected_files).unwrap_or("[]".into()))
    .bind(&now)
    .execute(&state.pool)
    .await
    .map_err(db_err)?;
    sqlx::query("UPDATE projects SET document_root=?1, updated_at=?2 WHERE id=?3")
        .bind(&info.recommended_document_root)
        .bind(&now)
        .bind(project_id)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    Ok(serde_json::to_value(&info).unwrap_or(json!({})))
}

pub async fn list_projects(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query("SELECT * FROM projects ORDER BY created_at DESC")
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;
    let mut out = Vec::new();
    for r in &rows {
        out.push(serde_json::to_value(project_out(&state.pool, r).await).unwrap());
    }
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct CreateProject {
    pub name: String,
    pub template: Option<String>,
}

pub async fn create_project(
    State(state): State<AppState>,
    Json(body): Json<CreateProject>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("Nama proyek wajib diisi"));
    }
    let template = body.template.unwrap_or_else(|| "static".to_string());
    if !crate::templates::TEMPLATES.contains(&template.as_str()) {
        return Err(ApiError::bad_request(format!("Template tidak dikenal: {}", template)));
    }
    let base_slug = slugify(&name);
    let mut slug = base_slug.clone();
    let mut n = 2;
    while state.config.workspace_dir.join(&slug).exists() {
        slug = format!("{}-{}", base_slug, n);
        n += 1;
    }
    let path = state.config.workspace_dir.join(&slug);
    crate::templates::write_template(&path, &template, &name)
        .map_err(|e| ApiError::internal(format!("Gagal membuat template: {}", e)))?;
    let id = new_id();
    let now = now_iso();
    sqlx::query("INSERT INTO projects (id, name, slug, path, document_root, created_at, updated_at) VALUES (?1,?2,?3,?4,'.',?5,?5)")
        .bind(&id)
        .bind(&name)
        .bind(&slug)
        .bind(path.to_string_lossy().to_string())
        .bind(&now)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    run_scan(&state, &id, &path.to_string_lossy()).await?;
    crate::state::app_log(&state.pool, "project", "info", &format!("Proyek dibuat: {}", name), None).await;
    let row = get_project_row(&state.pool, &id).await?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(project_out(&state.pool, &row).await).unwrap())))
}

#[derive(Deserialize)]
pub struct RegisterProject {
    pub name: String,
    pub path: String,
}

pub async fn register_project(
    State(state): State<AppState>,
    Json(body): Json<RegisterProject>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("Nama proyek wajib diisi"));
    }
    let raw = PathBuf::from(&body.path);
    if !raw.is_absolute() {
        return Err(ApiError::bad_request("Path harus absolut"));
    }
    let canon = raw
        .canonicalize()
        .map_err(|_| ApiError::bad_request("Path tidak ditemukan"))?;
    if !canon.is_dir() {
        return Err(ApiError::bad_request("Path bukan folder"));
    }
    if !is_inside(&state.config.workspace_dir, &canon) && !is_inside(&state.config.data_dir, &canon) {
        return Err(ApiError::bad_request("Path di luar workspace/app-specific dir tidak diizinkan"));
    }
    // already registered?
    let existing = sqlx::query("SELECT id FROM projects WHERE path=?1")
        .bind(canon.to_string_lossy().to_string())
        .fetch_optional(&state.pool)
        .await
        .map_err(db_err)?;
    if existing.is_some() {
        return Err(ApiError::conflict("Path sudah terdaftar sebagai proyek"));
    }
    let base_slug = slugify(&name);
    let mut slug = base_slug.clone();
    let mut n = 2;
    loop {
        let exists = sqlx::query("SELECT id FROM projects WHERE slug=?1")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await
            .map_err(db_err)?;
        if exists.is_none() {
            break;
        }
        slug = format!("{}-{}", base_slug, n);
        n += 1;
    }
    let id = new_id();
    let now = now_iso();
    sqlx::query("INSERT INTO projects (id, name, slug, path, document_root, created_at, updated_at) VALUES (?1,?2,?3,?4,'.',?5,?5)")
        .bind(&id)
        .bind(&name)
        .bind(&slug)
        .bind(canon.to_string_lossy().to_string())
        .bind(&now)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    run_scan(&state, &id, &canon.to_string_lossy()).await?;
    let row = get_project_row(&state.pool, &id).await?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(project_out(&state.pool, &row).await).unwrap())))
}

pub async fn get_project(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = get_project_row(&state.pool, &id).await?;
    Ok(Json(serde_json::to_value(project_out(&state.pool, &row).await).unwrap()))
}

#[derive(Deserialize, Default)]
pub struct DeleteProject {
    pub delete_files: Option<bool>,
}

pub async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<DeleteProject>>,
) -> ApiResult<Json<Value>> {
    let row = get_project_row(&state.pool, &id).await?;
    let running = sqlx::query("SELECT id FROM server_instances WHERE project_id=?1 AND status IN ('running','starting')")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db_err)?;
    if running.is_some() {
        return Err(ApiError::conflict("Hentikan server proyek ini sebelum menghapus proyek"));
    }
    let delete_files = body.and_then(|Json(b)| b.delete_files).unwrap_or(false);
    let project_path: String = row.get("path");
    sqlx::query("DELETE FROM projects WHERE id=?1")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    if delete_files {
        let p = PathBuf::from(&project_path);
        if is_inside(&state.config.workspace_dir, &p) && p.exists() {
            std::fs::remove_dir_all(&p).map_err(|e| ApiError::internal(format!("Gagal menghapus folder: {}", e)))?;
        }
    }
    Ok(Json(json!({"ok": true})))
}

pub async fn scan_project(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = get_project_row(&state.pool, &id).await?;
    let path: String = row.get("path");
    let info = run_scan(&state, &id, &path).await?;
    Ok(Json(info))
}
