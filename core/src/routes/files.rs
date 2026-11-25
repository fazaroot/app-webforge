use crate::state::AppState;
use crate::util::{iso_of, resolve_inside, ApiError, ApiResult};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::{Path, PathBuf};

async fn project_base(state: &AppState, project_id: &str) -> ApiResult<PathBuf> {
    let row = super::projects::get_project_row(&state.pool, project_id).await?;
    Ok(PathBuf::from(row.get::<String, _>("path")))
}

fn rel_string(base: &Path, target: &Path) -> String {
    target
        .strip_prefix(base.canonicalize().unwrap_or_else(|_| base.to_path_buf()))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn modified_iso(p: &Path) -> String {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .map(|t| iso_of(chrono::DateTime::<chrono::Utc>::from(t)))
        .unwrap_or_else(|_| crate::util::now_iso())
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub project_id: String,
    pub path: Option<String>,
}

pub async fn list_files(State(state): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &q.project_id).await?;
    let rel = q.path.unwrap_or_default();
    let dir = resolve_inside(&base, &rel, true)?;
    if !dir.is_dir() {
        return Err(ApiError::bad_request("Path bukan folder"));
    }
    let mut entries: Vec<Value> = Vec::new();
    let rd = std::fs::read_dir(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
    for e in rd.flatten() {
        let p = e.path();
        let meta = e.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        entries.push(json!({
            "name": e.file_name().to_string_lossy().to_string(),
            "path": rel_string(&base, &p),
            "is_dir": is_dir,
            "size": if is_dir { 0 } else { meta.map(|m| m.len()).unwrap_or(0) },
            "modified_at": modified_iso(&p),
        }));
    }
    entries.sort_by(|a, b| {
        let da = a["is_dir"].as_bool().unwrap_or(false);
        let db = b["is_dir"].as_bool().unwrap_or(false);
        db.cmp(&da).then(
            a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase()),
        )
    });
    Ok(Json(json!(entries)))
}

pub async fn read_file(State(state): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &q.project_id).await?;
    let rel = q.path.clone().unwrap_or_default();
    if rel.is_empty() {
        return Err(ApiError::bad_request("Parameter path wajib diisi"));
    }
    let p = resolve_inside(&base, &rel, true)?;
    if p.is_dir() {
        return Err(ApiError::bad_request("Path adalah folder"));
    }
    let meta = std::fs::metadata(&p).map_err(|e| ApiError::internal(e.to_string()))?;
    if meta.len() > 1024 * 1024 {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "File lebih besar dari 1MB"));
    }
    let bytes = std::fs::read(&p).map_err(|e| ApiError::internal(e.to_string()))?;
    if bytes.contains(&0) {
        return Err(ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "File biner tidak dapat dibaca sebagai teks"));
    }
    let content = String::from_utf8(bytes).map_err(|_| {
        ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "File biner tidak dapat dibaca sebagai teks")
    })?;
    Ok(Json(json!({"path": rel, "content": content, "size": meta.len()})))
}

#[derive(Deserialize)]
pub struct WriteBody {
    pub project_id: String,
    pub path: String,
    pub content: String,
}

pub async fn write_file(State(state): State<AppState>, Json(b): Json<WriteBody>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &b.project_id).await?;
    let p = resolve_inside(&base, &b.path, false)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    std::fs::write(&p, &b.content).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"ok": true, "size": b.content.len()})))
}

#[derive(Deserialize)]
pub struct PathBody {
    pub project_id: String,
    pub path: String,
}

pub async fn mkdir(State(state): State<AppState>, Json(b): Json<PathBody>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &b.project_id).await?;
    let p = resolve_inside(&base, &b.path, false)?;
    std::fs::create_dir_all(&p).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_file(State(state): State<AppState>, Json(b): Json<PathBody>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &b.project_id).await?;
    if b.path.is_empty() || b.path == "." {
        return Err(ApiError::bad_request("Tidak dapat menghapus root proyek"));
    }
    let p = resolve_inside(&base, &b.path, true)?;
    if p.is_dir() {
        std::fs::remove_dir_all(&p).map_err(|e| ApiError::internal(e.to_string()))?;
    } else {
        std::fs::remove_file(&p).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct FromToBody {
    pub project_id: String,
    pub from: String,
    pub to: String,
}

pub async fn rename_file(State(state): State<AppState>, Json(b): Json<FromToBody>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &b.project_id).await?;
    let from = resolve_inside(&base, &b.from, true)?;
    let to = resolve_inside(&base, &b.to, false)?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    std::fs::rename(&from, &to).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

pub async fn move_file(state: State<AppState>, body: Json<FromToBody>) -> ApiResult<Json<Value>> {
    rename_file(state, body).await
}

fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_recursive(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
        Ok(())
    }
}

pub async fn copy_file(State(state): State<AppState>, Json(b): Json<FromToBody>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &b.project_id).await?;
    let from = resolve_inside(&base, &b.from, true)?;
    let to = resolve_inside(&base, &b.to, false)?;
    copy_recursive(&from, &to).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub project_id: String,
    pub q: String,
}

pub async fn search_files(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &q.project_id).await?;
    let needle = q.q.to_lowercase();
    let mut out: Vec<Value> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        if out.len() >= 200 {
            break;
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = p.is_dir();
                if name.to_lowercase().contains(&needle) {
                    out.push(json!({"path": rel_string(&base, &p), "name": name, "is_dir": is_dir}));
                    if out.len() >= 200 {
                        break;
                    }
                }
                if is_dir {
                    stack.push(p);
                }
            }
        }
    }
    Ok(Json(json!(out)))
}

pub async fn dir_size(State(state): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let base = project_base(&state, &q.project_id).await?;
    let rel = q.path.clone().unwrap_or_default();
    let target = resolve_inside(&base, &rel, true)?;
    let mut bytes: u64 = 0;
    let mut files: u64 = 0;
    let mut stack: Vec<PathBuf> = vec![target.clone()];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                for e in rd.flatten() {
                    stack.push(e.path());
                }
            }
        } else {
            files += 1;
            bytes += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(Json(json!({"path": rel, "bytes": bytes, "files": files})))
}
