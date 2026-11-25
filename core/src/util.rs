use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::json;
use std::path::{Component, Path, PathBuf};

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn iso_of(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn slugify(name: &str) -> String {
    let mut s = String::new();
    let mut last_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !s.is_empty() {
            s.push('-');
            last_dash = true;
        }
    }
    while s.ends_with('-') {
        s.pop();
    }
    if s.is_empty() {
        "project".to_string()
    } else {
        s
    }
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, m)
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, m)
    }
    pub fn conflict(m: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, m)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn db_err(e: sqlx::Error) -> ApiError {
    ApiError::internal(format!("Database error: {}", e))
}

/// Resolve `rel` inside `base` (canonical). Rejects traversal with 403.
/// If must_exist, canonicalizes target; otherwise canonicalizes parent and joins file name.
pub fn resolve_inside(base: &Path, rel: &str, must_exist: bool) -> ApiResult<PathBuf> {
    let base_canon = base
        .canonicalize()
        .map_err(|e| ApiError::internal(format!("Base path tidak valid: {}", e)))?;
    let rel_path = Path::new(rel);
    // Reject absolute paths and parent components up front (still verified below by canonical check)
    for comp in rel_path.components() {
        match comp {
            Component::ParentDir => {
                return Err(ApiError::forbidden("Path traversal tidak diizinkan"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ApiError::forbidden("Path absolut tidak diizinkan"));
            }
            _ => {}
        }
    }
    let joined = if rel.is_empty() || rel == "." {
        base_canon.clone()
    } else {
        base_canon.join(rel_path)
    };
    let resolved = if must_exist {
        joined
            .canonicalize()
            .map_err(|_| ApiError::not_found(format!("Path tidak ditemukan: {}", rel)))?
    } else if joined.exists() {
        joined
            .canonicalize()
            .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        // canonicalize nearest existing ancestor, then append remaining components
        let mut ancestor = joined.clone();
        let mut rest: Vec<std::ffi::OsString> = Vec::new();
        loop {
            if ancestor.exists() {
                break;
            }
            match ancestor.file_name() {
                Some(name) => {
                    rest.push(name.to_os_string());
                    ancestor.pop();
                }
                None => return Err(ApiError::bad_request("Path tidak valid")),
            }
        }
        let mut canon = ancestor
            .canonicalize()
            .map_err(|e| ApiError::internal(e.to_string()))?;
        for part in rest.iter().rev() {
            canon.push(part);
        }
        canon
    };
    if !resolved.starts_with(&base_canon) {
        return Err(ApiError::forbidden("Path traversal tidak diizinkan"));
    }
    Ok(resolved)
}

pub fn is_inside(base: &Path, target: &Path) -> bool {
    let base_canon = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let target_canon = target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
    target_canon.starts_with(&base_canon)
}

/// Komposisi environment spawn terpusat (murni, tanpa efek samping):
/// `LD_LIBRARY_PATH` ke nativeLibraryDir bila ada (binary .so bundled
/// bisa membutuhkan library pendamping di folder yang sama), `HOME` ke
/// folder basis aplikasi, `TMPDIR` ke `<basis>/tmp`. Nilai
/// `LD_LIBRARY_PATH` lama dipertahankan di belakang path native.
pub fn compose_spawn_env(
    native_lib_dir: Option<&Path>,
    home: &Path,
    existing_ld_library_path: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(dir) = native_lib_dir {
        let dir_str = dir.to_string_lossy().to_string();
        let value = match existing_ld_library_path.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            Some(existing) if existing != dir_str => format!("{}:{}", dir_str, existing),
            _ => dir_str,
        };
        env.push(("LD_LIBRARY_PATH".to_string(), value));
    }
    env.push(("HOME".to_string(), home.to_string_lossy().to_string()));
    env.push((
        "TMPDIR".to_string(),
        home.join("tmp").to_string_lossy().to_string(),
    ));
    env
}

/// Environment spawn terpusat dari kondisi proses saat ini.
pub fn spawn_env(native_lib_dir: Option<&Path>, home: &Path) -> Vec<(String, String)> {
    let existing = std::env::var("LD_LIBRARY_PATH").ok();
    compose_spawn_env(native_lib_dir, home, existing.as_deref())
}

/// Terapkan environment spawn terpusat ke perintah anak: buat folder
/// `HOME`, `<HOME>/tmp`, dan cwd (bila diberikan) agar cwd selalu valid,
/// lalu set env + current_dir. Environment khusus pemanggil diterapkan
/// sesudah fungsi ini sehingga bisa menimpa nilai bawaan.
pub fn prepare_spawn(
    cmd: &mut tokio::process::Command,
    native_lib_dir: Option<&Path>,
    home: &Path,
    cwd: Option<&Path>,
) {
    let _ = std::fs::create_dir_all(home);
    let _ = std::fs::create_dir_all(home.join("tmp"));
    let work_dir = cwd.unwrap_or(home);
    let _ = std::fs::create_dir_all(work_dir);
    cmd.current_dir(work_dir);
    for (k, v) in spawn_env(native_lib_dir, home) {
        cmd.env(k, v);
    }
}

pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Cool Project!"), "my-cool-project");
        assert_eq!(slugify("  --A--  "), "a");
        assert_eq!(slugify("!!!"), "project");
    }

    #[test]
    fn spawn_env_composition() {
        let env = compose_spawn_env(
            Some(Path::new("/data/app/lib")),
            Path::new("/data/webforge"),
            None,
        );
        assert!(env.contains(&("LD_LIBRARY_PATH".to_string(), "/data/app/lib".to_string())));
        assert!(env.contains(&("HOME".to_string(), "/data/webforge".to_string())));
        assert!(env.contains(&("TMPDIR".to_string(), "/data/webforge/tmp".to_string())));

        let env = compose_spawn_env(
            Some(Path::new("/data/app/lib")),
            Path::new("/data/webforge"),
            Some("/usr/lib"),
        );
        assert!(env.contains(&(
            "LD_LIBRARY_PATH".to_string(),
            "/data/app/lib:/usr/lib".to_string()
        )));

        let env = compose_spawn_env(None, Path::new("/data/webforge"), None);
        assert!(!env.iter().any(|(k, _)| k == "LD_LIBRARY_PATH"));
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn resolve_inside_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("wf-util-test-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = resolve_inside(&dir, "../etc/passwd", false);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().status, StatusCode::FORBIDDEN);
        let ok = resolve_inside(&dir, "sub/file.txt", false).unwrap();
        assert!(ok.starts_with(dir.canonicalize().unwrap()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
