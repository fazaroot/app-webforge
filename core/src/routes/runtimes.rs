//! Endpoint manajer runtime Python & Node.js (v0.3.0).
//! Semua operasi menjalankan binary sungguhan; output dikembalikan apa
//! adanya. Instalasi dependensi hanya terjadi atas aksi pengguna.

use crate::state::AppState;
use crate::util::{ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

#[derive(Serialize)]
pub struct PythonRuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub pip_available: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct PythonProjectInfo {
    pub venv_exists: bool,
    pub venv_path: Option<String>,
    pub requirements_exists: bool,
    pub packages: Vec<InstalledPackage>,
    pub entry_kind: Option<String>,
    pub entry: Option<String>,
}

#[derive(Serialize)]
pub struct NodeProjectInfoOut {
    pub package_json_exists: bool,
    pub scripts: std::collections::BTreeMap<String, String>,
    pub entry: Option<String>,
    pub node_modules_exists: bool,
    pub package_manager: Option<String>,
    pub npm_available: bool,
}

#[derive(Serialize)]
pub struct InstallResult {
    pub ok: bool,
    pub output: String,
}

fn python_base_env(state: &AppState, project_id: &str, binary: &FsPath) -> Vec<(String, String)> {
    if crate::python_runtime::is_bundled_python(binary, state.native_lib_dir.as_deref()) {
        crate::python_runtime::python_env(&state.config.data_dir, project_id)
    } else {
        let mut e = vec![
            ("HOME".to_string(), state.config.data_dir.to_string_lossy().to_string()),
            ("TMPDIR".to_string(), state.config.data_dir.join("tmp").to_string_lossy().to_string()),
        ];
        if let Some(sp) = crate::python_runtime::venv_site_packages(&state.config.data_dir, project_id) {
            e.push(("PYTHONPATH".to_string(), sp.to_string_lossy().to_string()));
        }
        e
    }
}

async fn run_capture(
    program: &FsPath,
    args: &[String],
    cwd: Option<&FsPath>,
    envs: &[(String, String)],
    timeout: Duration,
) -> Result<(bool, String), String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    // Environment spawn terpusat lebih dulu; env pemanggil menimpa.
    let home = envs
        .iter()
        .find(|(k, _)| k == "HOME")
        .map(|(_, v)| PathBuf::from(v))
        .or_else(crate::state::data_dir)
        .unwrap_or_else(|| std::env::temp_dir().join("webforge-probe"));
    crate::util::prepare_spawn(
        &mut cmd,
        crate::state::native_lib_dir().as_deref(),
        &home,
        cwd,
    );
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("Perintah {} timeout", program.display()))?
        .map_err(|e| format!("Gagal menjalankan {}: {}", program.display(), e))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text.trim().to_string()))
}

pub async fn python_info(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let binary = crate::python_runtime::find_python_binary(state.native_lib_dir.as_deref());
    let info = match binary {
        None => PythonRuntimeInfo {
            available: false,
            version: None,
            executable: None,
            pip_available: false,
            error: Some("Binary Python tidak ditemukan".to_string()),
        },
        Some(binary) => {
            let envs = python_base_env(&state, "", &binary);
            let version_run = run_capture(
                &binary,
                &["--version".to_string()],
                None,
                &envs,
                Duration::from_secs(5),
            )
            .await;
            match version_run {
                Ok((true, out)) => {
                    let version = out
                        .lines()
                        .next()
                        .map(|l| l.trim().trim_start_matches("Python ").to_string());
                    let pip = run_capture(
                        &binary,
                        &["-m".to_string(), "pip".to_string(), "--version".to_string()],
                        None,
                        &envs,
                        Duration::from_secs(10),
                    )
                    .await;
                    let pip_available = matches!(pip, Ok((true, _)));
                    PythonRuntimeInfo {
                        available: true,
                        version,
                        executable: Some(binary.to_string_lossy().to_string()),
                        pip_available,
                        error: if pip_available {
                            None
                        } else {
                            Some("pip tidak tersedia (ensurepip gagal)".to_string())
                        },
                    }
                }
                Ok((false, out)) => PythonRuntimeInfo {
                    available: false,
                    version: None,
                    executable: Some(binary.to_string_lossy().to_string()),
                    pip_available: false,
                    error: Some(format!("Python gagal dijalankan: {}", out)),
                },
                Err(e) => PythonRuntimeInfo {
                    available: false,
                    version: None,
                    executable: Some(binary.to_string_lossy().to_string()),
                    pip_available: false,
                    error: Some(e),
                },
            }
        }
    };
    Ok(Json(serde_json::to_value(info).unwrap()))
}

async fn project_path(state: &AppState, project_id: &str) -> ApiResult<PathBuf> {
    let row = super::projects::get_project_row(&state.pool, project_id).await?;
    Ok(PathBuf::from(row.get::<String, _>("path")))
}

pub async fn project_python(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let project = project_path(&state, &id).await?;
    let venv = crate::python_runtime::venv_dir(&state.config.data_dir, &id);
    let requirements_exists = project.join("requirements.txt").is_file();
    let entry = crate::python_runtime::detect_python_entry(&project);
    let entry_kind = serde_json::to_value(entry.kind)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let mut packages = Vec::new();
    if let Some(binary) = crate::python_runtime::find_python_binary(state.native_lib_dir.as_deref()) {
        let envs = python_base_env(&state, &id, &binary);
        if let Ok((true, out)) = run_capture(
            &binary,
            &["-m".to_string(), "pip".to_string(), "list".to_string(), "--format=freeze".to_string()],
            Some(&project),
            &envs,
            Duration::from_secs(15),
        )
        .await
        {
            packages = crate::python_runtime::parse_pip_list(&out)
                .into_iter()
                .map(|(name, version)| InstalledPackage { name, version })
                .collect();
        }
    }
    let info = PythonProjectInfo {
        venv_exists: venv.is_dir(),
        venv_path: if venv.is_dir() {
            Some(venv.to_string_lossy().to_string())
        } else {
            None
        },
        requirements_exists,
        packages,
        entry_kind,
        entry: Some(entry.entry),
    };
    Ok(Json(serde_json::to_value(info).unwrap()))
}

pub async fn create_python_venv(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let project = project_path(&state, &id).await?;
    let binary = crate::python_runtime::find_python_binary(state.native_lib_dir.as_deref())
        .ok_or_else(|| ApiError::bad_request("Python tidak tersedia"))?;
    let venv = crate::python_runtime::venv_dir(&state.config.data_dir, &id);
    if let Some(parent) = venv.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let envs = python_base_env(&state, &id, &binary);
    let (ok, output) = run_capture(
        &binary,
        &crate::python_runtime::build_venv_args(&venv),
        Some(&project),
        &envs,
        Duration::from_secs(120),
    )
    .await
    .map_err(ApiError::bad_request)?;
    if !ok {
        // Fallback jujur: buat folder site-packages terisolasi; pip --target
        // akan memasang ke sana dan PYTHONPATH mengarah ke folder itu.
        let fallback = venv.join("site-packages");
        let _ = std::fs::create_dir_all(&fallback);
        let out = InstallResult {
            ok: false,
            output: format!(
                "Pembuatan venv standar gagal; folder site-packages terisolasi disiapkan sebagai fallback.\n{}",
                output
            ),
        };
        return Ok(Json(serde_json::to_value(out).unwrap()));
    }
    let out = InstallResult { ok: true, output };
    Ok(Json(serde_json::to_value(out).unwrap()))
}

pub async fn install_python_deps(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let project = project_path(&state, &id).await?;
    let binary = crate::python_runtime::find_python_binary(state.native_lib_dir.as_deref())
        .ok_or_else(|| ApiError::bad_request("Python tidak tersedia"))?;
    let requirements = project.join("requirements.txt");
    if !requirements.is_file() {
        return Err(ApiError::bad_request("requirements.txt tidak ditemukan di proyek"));
    }
    let venv = crate::python_runtime::venv_dir(&state.config.data_dir, &id);
    let target = crate::python_runtime::venv_site_packages(&state.config.data_dir, &id)
        .unwrap_or_else(|| venv.join("site-packages"));
    let _ = std::fs::create_dir_all(&target);
    let envs = python_base_env(&state, &id, &binary);
    let (ok, output) = run_capture(
        &binary,
        &crate::python_runtime::build_pip_install_args(&target, &requirements),
        Some(&project),
        &envs,
        Duration::from_secs(600),
    )
    .await
    .map_err(ApiError::bad_request)?;
    let out = InstallResult { ok, output };
    Ok(Json(serde_json::to_value(out).unwrap()))
}

fn npm_cli_path(state: &AppState) -> Option<PathBuf> {
    let npm_root = state.config.data_dir.join("node").join("npm");
    let cand = npm_root.join("package").join("bin").join("npm-cli.js");
    if cand.is_file() {
        return Some(cand);
    }
    // Ekstrak npm.zip (asset hasil build CI, root zip: package/...) sekali.
    let zip_path = state.config.data_dir.join("assets").join("npm.zip");
    if zip_path.is_file() {
        if extract_zip(&zip_path, &npm_root).is_ok() && cand.is_file() {
            return Some(cand);
        }
    }
    crate::runtime::find_in_path("npm")
}

fn extract_zip(zip_path: &FsPath, dest: &FsPath) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        let target = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub async fn project_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let project = project_path(&state, &id).await?;
    let info = crate::node_runtime::inspect_node_project(&project);
    let out = NodeProjectInfoOut {
        package_json_exists: project.join("package.json").is_file(),
        scripts: info.scripts,
        entry: info.entry,
        node_modules_exists: info.has_node_modules,
        package_manager: info.package_manager,
        npm_available: npm_cli_path(&state).is_some(),
    };
    Ok(Json(serde_json::to_value(out).unwrap()))
}

pub async fn install_node_deps(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let project = project_path(&state, &id).await?;
    if !project.join("package.json").is_file() {
        return Err(ApiError::bad_request("package.json tidak ditemukan di proyek"));
    }
    let npm_cli = npm_cli_path(&state).ok_or_else(|| {
        ApiError::bad_request(
            "npm tidak tersedia; sediakan folder node_modules secara manual",
        )
    })?;
    #[cfg(target_os = "android")]
    {
        // Jalankan npm-cli.js lewat NodeService (:node) dan tunggu status.
        let home = state.config.data_dir.join("node").join("install").join(&id);
        let _ = std::fs::create_dir_all(&home);
        let log_path = home.join("npm-install.log");
        let status_path = home.join("npm-install-status.txt");
        let _ = std::fs::write(&status_path, "starting");
        let args = vec![
            npm_cli.to_string_lossy().to_string(),
            "install".to_string(),
            "--no-audit".to_string(),
            "--no-fund".to_string(),
        ];
        let started = crate::jni_bridge::node_start(
            &args,
            &project.to_string_lossy(),
            &log_path.to_string_lossy(),
            &status_path.to_string_lossy(),
        );
        if !started {
            return Err(ApiError::bad_request("NodeService gagal dimulai untuk npm install"));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let status = std::fs::read_to_string(&status_path).unwrap_or_default();
            let status = status.trim();
            if let Ok(code) = status.parse::<i32>() {
                let output = std::fs::read_to_string(&log_path).unwrap_or_default();
                let out = InstallResult { ok: code == 0, output };
                return Ok(Json(serde_json::to_value(out).unwrap()));
            }
            if tokio::time::Instant::now() > deadline {
                let _ = crate::jni_bridge::node_stop();
                return Err(ApiError::bad_request("npm install timeout setelah 600 detik"));
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let node = crate::runtime::find_in_path("node")
            .ok_or_else(|| ApiError::bad_request("Node.js tidak tersedia"))?;
        let (ok, output) = if npm_cli.file_name().map(|n| n == "npm-cli.js").unwrap_or(false) {
            run_capture(
                &node,
                &[
                    npm_cli.to_string_lossy().to_string(),
                    "install".to_string(),
                    "--no-audit".to_string(),
                    "--no-fund".to_string(),
                ],
                Some(&project),
                &[],
                Duration::from_secs(600),
            )
            .await
            .map_err(ApiError::bad_request)?
        } else {
            run_capture(
                &npm_cli,
                &["install".to_string(), "--no-audit".to_string(), "--no-fund".to_string()],
                Some(&project),
                &[],
                Duration::from_secs(600),
            )
            .await
            .map_err(ApiError::bad_request)?
        };
        let out = InstallResult { ok, output };
        Ok(Json(serde_json::to_value(out).unwrap()))
    }
}
