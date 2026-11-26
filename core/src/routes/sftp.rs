//! Handler API SFTP sesuai kontrak v0.5.0. Registrasi route dilakukan
//! koordinator di `routes/mod.rs`:
//!
//! ```ignore
//! .route("/api/sftp/status", get(sftp::status))
//! .route("/api/sftp/settings", put(sftp::put_settings))
//! .route("/api/sftp/start", post(sftp::start))
//! .route("/api/sftp/stop", post(sftp::stop))
//! .route("/api/sftp/reset-password", post(sftp::reset_password))
//! ```
//!
//! Status tidak pernah memuat password/hash. Password plaintext hanya
//! dikembalikan sekali oleh reset-password.

use crate::sftp::{self, SftpSettings};
use crate::state::AppState;
use crate::util::{ApiError, ApiResult};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

fn status_json(status: sftp::SftpStatus) -> Json<Value> {
    Json(serde_json::to_value(status).unwrap_or(json!({})))
}

pub async fn status(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let status = state
        .sftp
        .status(&state.pool, &state.config.workspace_dir, &state.config.data_dir)
        .await
        .map_err(ApiError::internal)?;
    Ok(status_json(status))
}

#[derive(Deserialize)]
pub struct PutSettingsBody {
    pub username: String,
    /// Kosong/tidak diisi berarti pertahankan password lama.
    pub password: Option<String>,
    pub port: u16,
    pub bind_lan: bool,
}

pub async fn put_settings(
    State(state): State<AppState>,
    Json(body): Json<PutSettingsBody>,
) -> ApiResult<Json<Value>> {
    sftp::validate_settings(&body.username, body.port).map_err(ApiError::bad_request)?;
    let current = sftp::load_settings(&state.pool)
        .await
        .map_err(ApiError::internal)?;
    let password_hash = match body.password {
        Some(p) if !p.is_empty() => Some(sftp::hash_password(&p).map_err(ApiError::internal)?),
        _ => current.password_hash.clone(),
    };
    if password_hash.is_none() {
        return Err(ApiError::bad_request(
            "Password SFTP wajib diisi karena belum ada password tersimpan",
        ));
    }
    let settings = SftpSettings {
        username: body.username.trim().to_string(),
        password_hash,
        port: body.port,
        bind_lan: body.bind_lan,
    };
    let status = state
        .sftp
        .apply_settings(
            &state.pool,
            &state.config.workspace_dir,
            &state.config.data_dir,
            settings,
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    Ok(status_json(status))
}

pub async fn start(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let status = state
        .sftp
        .start(&state.pool, &state.config.workspace_dir, &state.config.data_dir)
        .await
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    Ok(status_json(status))
}

pub async fn stop(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let status = state
        .sftp
        .stop(&state.pool, &state.config.workspace_dir, &state.config.data_dir)
        .await
        .map_err(ApiError::internal)?;
    Ok(status_json(status))
}

pub async fn reset_password(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let password = state
        .sftp
        .reset_password(
            &state.pool,
            &state.config.workspace_dir,
            &state.config.data_dir,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "password": password })))
}
