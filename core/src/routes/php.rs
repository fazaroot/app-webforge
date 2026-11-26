use crate::php::{self, PhpSettings};
use crate::state::AppState;
use crate::util::{db_err, ApiError, ApiResult};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::time::{Duration, Instant};

const PHP_INFO_CACHE_TTL: Duration = Duration::from_secs(60);
const PHP_CLI_TIMEOUT: Duration = Duration::from_secs(30);
const COMPOSER_TIMEOUT: Duration = Duration::from_secs(600);
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

/// Info PHP multi-versi dari cache (probe semua versi + Composer sekali
/// per TTL). Pilihan versi per request diterapkan di atas cache.
pub async fn php_info_cached(state: &AppState) -> php::PhpInfo {
    {
        let cache = state.php_info_cache.lock().await;
        if let Some((at, info)) = cache.as_ref() {
            if at.elapsed() < PHP_INFO_CACHE_TTL {
                return info.clone();
            }
        }
    }
    let versions = php::probe_all_php_versions(state.native_lib_dir.as_deref()).await;
    let default_binary = php::default_php_version(&versions)
        .and_then(|v| versions.iter().find(|e| e.version == v))
        .and_then(|e| e.binary_path.clone())
        .map(std::path::PathBuf::from);
    let composer = php::probe_composer(&state.config.data_dir, default_binary.as_deref()).await;
    let info = php::PhpInfo::from_versions(versions, composer, None);
    {
        let mut cache = state.php_info_cache.lock().await;
        *cache = Some((Instant::now(), info.clone()));
    }
    info
}

#[derive(Deserialize)]
pub struct PhpInfoQuery {
    pub version: Option<String>,
}

pub async fn php_info(
    State(state): State<AppState>,
    Query(q): Query<PhpInfoQuery>,
) -> Json<Value> {
    let info = php_info_cached(&state).await;
    let selected = info.select(q.version.as_deref());
    Json(serde_json::to_value(&selected).unwrap_or(json!({})))
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
    let smtp = super::mail::load_smtp_config(&state.pool, &state.config.data_dir).await;
    php::write_php_ini_with_smtp(&state.config.data_dir, &body, smtp.as_ref())
        .map_err(|e| ApiError::internal(format!("Gagal menulis php.ini: {}", e)))?;
    let saved = php::load_php_settings(&state.pool).await;
    Ok(Json(serde_json::to_value(&saved).unwrap_or(json!({}))))
}

/// Versi PHP efektif proyek: pilihan proyek, atau None (aturan default).
fn project_php_version(project: &sqlx::sqlite::SqliteRow) -> Option<String> {
    project
        .try_get::<Option<String>, _>("php_version")
        .ok()
        .flatten()
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
    let resolved = php::resolve_php(
        state.native_lib_dir.as_deref(),
        project_php_version(&project).as_deref(),
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    // Pastikan php.ini terbaru ada agar PHPRC menerapkannya ke CLI.
    let settings = php::load_php_settings(&state.pool).await;
    let smtp = super::mail::load_smtp_config(&state.pool, &state.config.data_dir).await;
    php::write_php_ini_with_smtp(&state.config.data_dir, &settings, smtp.as_ref())
        .map_err(|e| ApiError::internal(format!("Gagal menulis php.ini: {}", e)))?;

    let project_path: String = project.get("path");
    let mut cmd = tokio::process::Command::new(&resolved.binary);
    cmd.args(&body.args).kill_on_drop(true);
    crate::util::prepare_spawn(
        &mut cmd,
        state.native_lib_dir.as_deref(),
        &state.config.data_dir,
        Some(std::path::Path::new(&project_path)),
    );
    cmd.env("TMPDIR", php::php_tmp_dir(&state.config.data_dir))
        .env("PHPRC", &state.config.data_dir);
    if php::icu_data_path(&state.config.data_dir).is_file() {
        cmd.env("ICU_DATA", state.config.data_dir.join("assets"));
    }
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

// ---------------------------------------------------------------------------
// Composer per proyek
// ---------------------------------------------------------------------------

/// Validasi nama paket Composer ketat: vendor/nama dengan huruf kecil,
/// angka, titik, garis bawah, strip. Constraint opsional setelah ':'
/// hanya karakter constraint yang aman.
pub fn validate_composer_package(package: &str) -> Result<String, String> {
    let package = package.trim();
    let (name, constraint) = match package.split_once(':') {
        Some((n, c)) => (n, Some(c)),
        None => (package, None),
    };
    let mut parts = name.split('/');
    let vendor = parts.next().unwrap_or("");
    let pkg = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return Err(format!("Nama paket '{}' tidak valid (format: vendor/nama)", package));
    }
    let valid_part = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.chars().next().map(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).unwrap_or(false)
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-')
    };
    if !valid_part(vendor) || !valid_part(pkg) {
        return Err(format!(
            "Nama paket '{}' tidak valid (format: vendor/nama, huruf kecil/angka/./_/-)",
            package
        ));
    }
    if let Some(c) = constraint {
        if c.is_empty()
            || c.len() > 64
            || !c.chars().all(|ch| {
                ch.is_ascii_alphanumeric()
                    || matches!(ch, '.' | '^' | '~' | '<' | '>' | '=' | '|' | ',' | '*' | '-' | ' ')
            })
        {
            return Err(format!("Constraint versi '{}' tidak valid", c));
        }
    }
    Ok(package.to_string())
}

async fn recommended_php_for_project(
    state: &AppState,
    project_id: &str,
    project_path: &str,
) -> Option<String> {
    let stored: Option<Option<String>> = sqlx::query_scalar(
        "SELECT recommended_php_version FROM project_frameworks WHERE project_id=?1 ORDER BY detected_at DESC LIMIT 1",
    )
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    if let Some(v) = stored.flatten() {
        return Some(v);
    }
    crate::framework::composer_php_constraint(std::path::Path::new(project_path))
        .and_then(|c| crate::framework::recommended_php_version(&c))
}

pub async fn project_composer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let project = super::projects::get_project_row(&state.pool, &id).await?;
    let project_path: String = project.get("path");
    let root = std::path::Path::new(&project_path);
    let info = php_info_cached(&state).await;
    let recommended = recommended_php_for_project(&state, &id, &project_path).await;
    let effective_version = project_php_version(&project)
        .or_else(|| info.selected_version.clone());
    Ok(Json(json!({
        "available": info.composer.available,
        "version": info.composer.version,
        "has_composer_json": root.join("composer.json").is_file(),
        "has_lock": root.join("composer.lock").is_file(),
        "vendor_present": root.join("vendor").is_dir(),
        "php_version": effective_version,
        "recommended_php_version": recommended,
        "error": info.composer.error,
    })))
}

#[derive(Deserialize)]
pub struct ComposerRunBody {
    pub action: String,
    pub package: Option<String>,
    pub php_version: Option<String>,
}

pub async fn composer_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ComposerRunBody>,
) -> ApiResult<Json<Value>> {
    let started = Instant::now();
    let project = super::projects::get_project_row(&state.pool, &id).await?;
    let project_path: String = project.get("path");

    let mut args: Vec<String> = match body.action.as_str() {
        "install" => vec!["install".to_string()],
        "update" => vec!["update".to_string()],
        "require" => {
            let package = body
                .package
                .as_deref()
                .ok_or_else(|| ApiError::bad_request("package wajib diisi untuk action require"))?;
            let package = validate_composer_package(package).map_err(ApiError::bad_request)?;
            vec!["require".to_string(), package]
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "action '{}' tidak dikenal (install, update, require)",
                other
            )))
        }
    };
    args.push("--no-interaction".to_string());
    args.push("--no-progress".to_string());

    let requested_version = match &body.php_version {
        Some(v) => php::validate_php_version_choice(Some(v)).map_err(ApiError::bad_request)?,
        None => project_php_version(&project),
    };
    let resolved = php::resolve_php(state.native_lib_dir.as_deref(), requested_version.as_deref())
        .await
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    let phar = php::composer_phar_path(&state.config.data_dir);
    if !phar.is_file() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Composer tidak tersedia: composer.phar tidak ditemukan di data/assets",
        ));
    }

    let settings = php::load_php_settings(&state.pool).await;
    let smtp = super::mail::load_smtp_config(&state.pool, &state.config.data_dir).await;
    php::write_php_ini_with_smtp(&state.config.data_dir, &settings, smtp.as_ref())
        .map_err(|e| ApiError::internal(format!("Gagal menulis php.ini: {}", e)))?;

    let composer_home = state.config.data_dir.join("composer-home");
    let _ = std::fs::create_dir_all(&composer_home);
    let mut cmd = tokio::process::Command::new(&resolved.binary);
    cmd.arg(&phar).args(&args).kill_on_drop(true);
    crate::util::prepare_spawn(
        &mut cmd,
        state.native_lib_dir.as_deref(),
        &state.config.data_dir,
        Some(std::path::Path::new(&project_path)),
    );
    cmd.env("TMPDIR", php::php_tmp_dir(&state.config.data_dir))
        .env("PHPRC", &state.config.data_dir)
        .env("COMPOSER_HOME", &composer_home)
        .env("COMPOSER_NO_INTERACTION", "1")
        .env("COMPOSER_ALLOW_SUPERUSER", "1");
    if php::icu_data_path(&state.config.data_dir).is_file() {
        cmd.env("ICU_DATA", state.config.data_dir.join("assets"));
    }

    match tokio::time::timeout(COMPOSER_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => Ok(Json(json!({
            "exit_code": out.status.code(),
            "stdout": truncate_output(&String::from_utf8_lossy(&out.stdout)),
            "stderr": truncate_output(&String::from_utf8_lossy(&out.stderr)),
            "timed_out": false,
            "duration_ms": started.elapsed().as_millis() as u64,
        }))),
        Ok(Err(e)) => Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Composer tidak dapat dijalankan: {}", e),
        )),
        Err(_) => Ok(Json(json!({
            "exit_code": null,
            "stdout": "",
            "stderr": "",
            "timed_out": true,
            "duration_ms": started.elapsed().as_millis() as u64,
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_package_validation() {
        assert!(validate_composer_package("laravel/framework").is_ok());
        assert!(validate_composer_package("guzzlehttp/guzzle:^7.0").is_ok());
        assert!(validate_composer_package("symfony/console:>=6.4 <8.0").is_ok());
        assert!(validate_composer_package("paket-tanpa-vendor").is_err());
        assert!(validate_composer_package("Vendor/Paket").is_err());
        assert!(validate_composer_package("vendor/paket; rm -rf /").is_err());
        assert!(validate_composer_package("vendor/paket:constraint$(jahat)").is_err());
        assert!(validate_composer_package("a/b/c").is_err());
        assert!(validate_composer_package("-vendor/paket").is_err());
        assert!(validate_composer_package("").is_err());
    }
}
