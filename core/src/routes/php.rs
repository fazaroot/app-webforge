use crate::php::{self, PhpSettings};
use crate::state::AppState;
use crate::util::{db_err, ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::time::{Duration, Instant};

const PHP_INFO_CACHE_TTL: Duration = Duration::from_secs(60);
const PHP_CLI_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_CHARS: usize = 262_144;

fn truncate_output(s: &str) -> String {
    if s.chars().count() <= MAX_OUTPUT_CHARS {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(MAX_OUTPUT_CHARS).collect();
        out.push_str("\n[output dipotong oleh WebForge]");
        out
    }
}

pub async fn php_info(State(state): State<AppState>) -> Json<Value> {
    {
        let cache = state.php_info_cache.lock().await;
        if let Some((at, info)) = cache.as_ref() {
            if at.elapsed() < PHP_INFO_CACHE_TTL {
                return Json(serde_json::to_value(info).unwrap_or(json!({})));
            }
        }
    }
    let binary = php::find_php_binary(state.native_lib_dir.as_deref());
    let info = php::probe_php_info(binary).await;
    {
        let mut cache = state.php_info_cache.lock().await;
        *cache = Some((Instant::now(), info.clone()));
    }
    Json(serde_json::to_value(&info).unwrap_or(json!({})))
}

pub async fn get_php_settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let settings = php::load_php_settings(&state.pool).await;
    Ok(Json(serde_json::to_value(&settings).unwrap_or(json!({}))))
}

pub async fn put_php_settings(
    State(state): State<AppState>,
    Json(body): Json<PhpSettings>,
) -> ApiResult<Json<Value>> {
    body.validate().map_err(ApiError::bad_request)?;
    php::save_php_settings(&state.pool, &body).await.map_err(db_err)?;
    php::write_php_ini(&state.config.data_dir, &body)
        .map_err(|e| ApiError::internal(format!("Gagal menulis php.ini: {}", e)))?;
    let saved = php::load_php_settings(&state.pool).await;
    Ok(Json(serde_json::to_value(&saved).unwrap_or(json!({}))))
}

#[derive(Deserialize)]
pub struct PhpCliBody {
    pub args: Vec<String>,
}

pub async fn php_cli(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PhpCliBody>,
) -> ApiResult<Json<Value>> {
    let project = super::projects::get_project_row(&state.pool, &id).await?;
    if body.args.is_empty() {
        return Err(ApiError::bad_request("args tidak boleh kosong"));
    }
    let binary = php::find_php_binary(state.native_lib_dir.as_deref()).ok_or_else(|| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "PHP tidak tersedia: libphp.so tidak ditemukan di nativeLibraryDir dan php tidak ada di PATH",
        )
    })?;
    // Pastikan php.ini terbaru ada agar PHPRC menerapkannya ke CLI.
    let settings = php::load_php_settings(&state.pool).await;
    php::write_php_ini(&state.config.data_dir, &settings)
        .map_err(|e| ApiError::internal(format!("Gagal menulis php.ini: {}", e)))?;

    let project_path: String = project.get("path");
    let mut cmd = tokio::process::Command::new(&binary);
    cmd.args(&body.args).kill_on_drop(true);
    crate::util::prepare_spawn(
        &mut cmd,
        state.native_lib_dir.as_deref(),
        &state.config.data_dir,
        Some(std::path::Path::new(&project_path)),
    );
    cmd.env("TMPDIR", php::php_tmp_dir(&state.config.data_dir))
        .env("PHPRC", &state.config.data_dir);
    match tokio::time::timeout(PHP_CLI_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => Ok(Json(json!({
            "exit_code": out.status.code(),
            "stdout": truncate_output(&String::from_utf8_lossy(&out.stdout)),
            "stderr": truncate_output(&String::from_utf8_lossy(&out.stderr)),
            "timed_out": false,
        }))),
        Ok(Err(e)) => Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("PHP tidak dapat dijalankan: {}", e),
        )),
        Err(_) => Ok(Json(json!({
            "exit_code": null,
            "stdout": "",
            "stderr": "",
            "timed_out": true,
        }))),
    }
}
