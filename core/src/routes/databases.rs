use crate::db_runtime;
use crate::state::{AppState, ProcessHandle};
use crate::util::{db_err, is_inside, new_id, now_iso, ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Column, Row, TypeInfo};
use std::path::{Path as FsPath, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

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

fn kind_of(row: &sqlx::sqlite::SqliteRow) -> String {
    row.try_get::<String, _>("kind").unwrap_or_else(|_| "sqlite".to_string())
}

fn db_out(row: &sqlx::sqlite::SqliteRow) -> Value {
    let path: String = row.get("path");
    let kind = kind_of(row);
    let size = if kind == "sqlite" {
        std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0)
    } else {
        0
    };
    json!({
        "id": row.get::<String, _>("id"),
        "name": row.get::<String, _>("name"),
        "path": path,
        "size_bytes": size,
        "created_at": row.get::<String, _>("created_at"),
        "kind": kind,
        "status": row.try_get::<String, _>("status").unwrap_or_else(|_| "stopped".to_string()),
        "host": row.try_get::<Option<String>, _>("host").ok().flatten(),
        "port": row.try_get::<Option<i64>, _>("port").ok().flatten(),
        "username": row.try_get::<Option<String>, _>("username").ok().flatten(),
        "database_name": row.try_get::<Option<String>, _>("database_name").ok().flatten(),
        "error_message": row.try_get::<Option<String>, _>("error_message").ok().flatten(),
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
    pub kind: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database_name: Option<String>,
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub async fn create_database(State(state): State<AppState>, Json(b): Json<CreateDb>) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = b.name.trim().to_string();
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request("Nama database hanya boleh huruf, angka, '-' dan '_'"));
    }
    let kind = b.kind.as_deref().unwrap_or("sqlite");
    let id = new_id();
    let now = now_iso();
    match kind {
        "sqlite" => {
            let path = state.config.databases_dir().join(format!("{}.db", name));
            if path.exists() {
                return Err(ApiError::conflict("Database dengan nama itu sudah ada"));
            }
            let pool = open_db(&path.to_string_lossy()).await?;
            sqlx::query("PRAGMA user_version").execute(&pool).await.map_err(db_err)?;
            pool.close().await;
            let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
            sqlx::query("INSERT INTO database_instances (id, name, path, size_bytes, created_at, kind, status) VALUES (?1,?2,?3,?4,?5,'sqlite','stopped')")
                .bind(&id)
                .bind(&name)
                .bind(path.to_string_lossy().to_string())
                .bind(size)
                .bind(&now)
                .execute(&state.pool)
                .await
                .map_err(db_err)?;
        }
        "postgresql" | "mariadb" => {
            let runtimes = crate::runtime::runtimes_cached(&state, false).await;
            let available = runtimes.iter().any(|r| r.id == kind && r.available);
            if !available {
                return Err(ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("Runtime {} tidak tersedia", kind),
                ));
            }
            let default_port = if kind == "postgresql" {
                db_runtime::POSTGRES_DEFAULT_PORT as i64
            } else {
                db_runtime::MARIADB_DEFAULT_PORT as i64
            };
            let port = b.port.unwrap_or(default_port);
            if !(1024..=65535).contains(&port) {
                return Err(ApiError::bad_request("Port harus antara 1024 dan 65535"));
            }
            if !port_free(port as u16) {
                return Err(ApiError::conflict(format!("Port {} sudah digunakan", port)));
            }
            let username = b
                .username
                .clone()
                .filter(|u| !u.trim().is_empty())
                .unwrap_or_else(|| if kind == "postgresql" { "postgres".to_string() } else { "root".to_string() });
            db_runtime::validate_db_identifier(&username).map_err(ApiError::bad_request)?;
            let database_name = b
                .database_name
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| name.replace('-', "_"));
            db_runtime::validate_db_identifier(&database_name).map_err(ApiError::bad_request)?;
            let password = crate::secrets::generate_password();
            let encrypted = crate::secrets::encrypt_secret(&state.config.data_dir, &password)
                .map_err(ApiError::internal)?;
            let home = if kind == "postgresql" {
                db_runtime::postgres_home(&state.config.data_dir, &id)
            } else {
                db_runtime::mariadb_home(&state.config.data_dir, &id)
            };
            sqlx::query(
                "INSERT INTO database_instances (id, name, path, size_bytes, created_at, kind, host, port, username, password_encrypted, database_name, status, data_dir)
                 VALUES (?1,?2,?3,0,?4,?5,'127.0.0.1',?6,?7,?8,?9,'stopped',?3)",
            )
            .bind(&id)
            .bind(&name)
            .bind(home.to_string_lossy().to_string())
            .bind(&now)
            .bind(kind)
            .bind(port)
            .bind(&username)
            .bind(&encrypted)
            .bind(&database_name)
            .execute(&state.pool)
            .await
            .map_err(db_err)?;
        }
        "external-mysql" | "external-postgresql" => {
            let host = b.host.clone().filter(|h| !h.trim().is_empty()).ok_or_else(|| ApiError::bad_request("Host wajib diisi"))?;
            let username = b.username.clone().filter(|u| !u.trim().is_empty()).ok_or_else(|| ApiError::bad_request("Username wajib diisi"))?;
            let database_name = b.database_name.clone().filter(|d| !d.trim().is_empty()).ok_or_else(|| ApiError::bad_request("Nama database wajib diisi"))?;
            let default_port = if kind == "external-postgresql" { 5432 } else { 3306 };
            let port = b.port.unwrap_or(default_port);
            if !(1..=65535).contains(&port) {
                return Err(ApiError::bad_request("Port tidak valid"));
            }
            let encrypted = match b.password.as_deref().filter(|p| !p.is_empty()) {
                Some(pw) => Some(
                    crate::secrets::encrypt_secret(&state.config.data_dir, pw)
                        .map_err(ApiError::internal)?,
                ),
                None => None,
            };
            sqlx::query(
                "INSERT INTO database_instances (id, name, path, size_bytes, created_at, kind, host, port, username, password_encrypted, database_name, status)
                 VALUES (?1,?2,'',0,?3,?4,?5,?6,?7,?8,?9,'stopped')",
            )
            .bind(&id)
            .bind(&name)
            .bind(&now)
            .bind(kind)
            .bind(&host)
            .bind(port)
            .bind(&username)
            .bind(&encrypted)
            .bind(&database_name)
            .execute(&state.pool)
            .await
            .map_err(db_err)?;
        }
        other => {
            return Err(ApiError::bad_request(format!("Kind database {} tidak dikenal", other)));
        }
    }
    let row = db_row(&state, &id).await?;
    Ok((StatusCode::CREATED, Json(db_out(&row))))
}

fn decrypted_password(state: &AppState, row: &sqlx::sqlite::SqliteRow) -> Option<String> {
    let enc: Option<String> = row.try_get("password_encrypted").ok().flatten();
    enc.and_then(|e| crate::secrets::decrypt_secret(&state.config.data_dir, &e).ok())
}

#[derive(Deserialize)]
pub struct TestConnectionBody {
    pub kind: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub password: Option<String>,
    pub database_name: String,
}

async fn pg_connect(host: &str, port: i64, username: &str, password: &str, database: &str) -> Result<sqlx::PgPool, String> {
    let url = format!(
        "postgres://{}:{}@{}:{}/{}",
        urlencoding(username),
        urlencoding(password),
        host,
        port,
        urlencoding(database)
    );
    let opts = sqlx::postgres::PgConnectOptions::from_str(&url)
        .map_err(|e| e.to_string())?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(opts)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("SELECT 1").execute(&pool).await.map_err(|e| e.to_string())?;
    Ok(pool)
}

async fn mysql_connect(host: &str, port: i64, username: &str, password: &str, database: &str) -> Result<sqlx::MySqlPool, String> {
    let url = format!(
        "mysql://{}:{}@{}:{}/{}",
        urlencoding(username),
        urlencoding(password),
        host,
        port,
        urlencoding(database)
    );
    let opts = sqlx::mysql::MySqlConnectOptions::from_str(&url)
        .map_err(|e| e.to_string())?;
    let pool = sqlx::mysql::MySqlPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(opts)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("SELECT 1").execute(&pool).await.map_err(|e| e.to_string())?;
    Ok(pool)
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub async fn test_connection(State(_state): State<AppState>, Json(b): Json<TestConnectionBody>) -> ApiResult<Json<Value>> {
    let password = b.password.as_deref().unwrap_or("");
    let result = match b.kind.as_str() {
        "external-postgresql" | "postgresql" => {
            pg_connect(&b.host, b.port, &b.username, password, &b.database_name)
                .await
                .map(|_| ())
        }
        "external-mysql" | "mariadb" => {
            mysql_connect(&b.host, b.port, &b.username, password, &b.database_name)
                .await
                .map(|_| ())
        }
        other => Err(format!("Kind {} tidak mendukung tes koneksi", other)),
    };
    match result {
        Ok(_) => Ok(Json(json!({"ok": true, "message": "Koneksi berhasil"}))),
        Err(e) => Ok(Json(json!({"ok": false, "message": e}))),
    }
}

async fn set_db_status(state: &AppState, id: &str, status: &str, error: Option<&str>) {
    let _ = sqlx::query("UPDATE database_instances SET status=?1, error_message=?2 WHERE id=?3")
        .bind(status)
        .bind(error)
        .bind(id)
        .execute(&state.pool)
        .await;
}

fn extract_zip(zip_path: &FsPath, dest: &FsPath) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("Asset {} tidak ditemukan: {}", zip_path.display(), e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Asset {} rusak: {}", zip_path.display(), e))?;
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

pub(crate) fn ensure_share(state: &AppState, kind: &str, home: &FsPath) -> Result<PathBuf, String> {
    // Layout zip berbeda: postgres-share.zip berakar share/postgresql/...
    // (diekstrak ke home), mariadb-share.zip berakar isi share itu sendiri
    // (diekstrak ke share dir).
    let (zip_name, share_dir, extract_dest, marker) = if kind == "postgresql" {
        (
            "postgres-share.zip",
            db_runtime::postgres_share_dir(home),
            home.to_path_buf(),
            "postgres.bki",
        )
    } else {
        let share = db_runtime::mariadb_share_dir(home);
        (
            "mariadb-share.zip",
            share.clone(),
            share,
            "mysql_system_tables.sql",
        )
    };
    if share_dir.join(marker).is_file() {
        return Ok(share_dir);
    }
    let zip_path = state.config.data_dir.join("assets").join(zip_name);
    extract_zip(&zip_path, &extract_dest)?;
    if !share_dir.join(marker).is_file() {
        return Err(format!(
            "Ekstrak {} selesai tetapi {} tidak ditemukan",
            zip_name, marker
        ));
    }
    Ok(share_dir)
}

async fn run_command(
    program: &FsPath,
    args: &[String],
    stdin_data: Option<&str>,
    timeout: Duration,
) -> Result<(bool, String), String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    let home = crate::state::data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("webforge-probe"));
    crate::util::prepare_spawn(
        &mut cmd,
        crate::state::native_lib_dir().as_deref(),
        &home,
        Some(&home),
    );
    if stdin_data.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Gagal menjalankan {}: {}", program.display(), e))?;
    if let Some(data) = stdin_data {
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
    }
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("{} timeout", program.display()))?
        .map_err(|e| e.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text.trim().to_string()))
}

async fn spawn_service(
    state: &AppState,
    id: &str,
    program: PathBuf,
    args: Vec<String>,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args);
    crate::util::prepare_spawn(
        &mut cmd,
        state.native_lib_dir.as_deref(),
        &state.config.data_dir,
        Some(&state.config.data_dir),
    );
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Gagal menjalankan {}: {}", program.display(), e))?;
    // Tangkap output ke log center.
    let pool = state.pool.clone();
    if let Some(stdout) = child.stdout.take() {
        let pool = pool.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    crate::state::app_log(&pool, "database", "info", &line, None).await;
                }
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let pool = pool.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    crate::state::app_log(&pool, "database", "info", &line, None).await;
                }
            }
        });
    }
    state
        .processes
        .lock()
        .await
        .insert(format!("db:{}", id), ProcessHandle::Child(Arc::new(Mutex::new(Some(child)))));
    Ok(())
}

async fn wait_pg_ready(port: i64, username: &str, password: &str) -> bool {
    for _ in 0..60 {
        if pg_connect("127.0.0.1", port, username, password, "postgres").await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn wait_mysql_ready(port: i64, username: &str, password: &str) -> bool {
    for _ in 0..60 {
        if mysql_connect("127.0.0.1", port, username, password, "mysql").await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn start_postgres(state: &AppState, row: &sqlx::sqlite::SqliteRow) -> Result<(), String> {
    let id: String = row.get("id");
    let port: i64 = row.try_get("port").ok().flatten().unwrap_or(5432);
    let username: String = row.try_get("username").ok().flatten().unwrap_or_else(|| "postgres".to_string());
    let database_name: String = row.try_get("database_name").ok().flatten().unwrap_or_else(|| "postgres".to_string());
    let password = decrypted_password(state, row).ok_or("Password database lokal tidak dapat didekripsi")?;
    let home = db_runtime::postgres_home(&state.config.data_dir, &id);
    let native_dir = state
        .native_lib_dir
        .clone()
        .ok_or("nativeLibraryDir tidak diketahui")?;
    for dir in db_runtime::postgres_required_dirs(&home) {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    for (link, target) in db_runtime::postgres_symlinks(&home, &native_dir) {
        if !link.exists() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link).map_err(|e| e.to_string())?;
        }
    }
    let share = ensure_share(state, "postgresql", &home)?;
    let data_dir = db_runtime::postgres_data_dir(&home);
    if !data_dir.join("PG_VERSION").is_file() {
        let pwfile = home.join("pwfile");
        std::fs::write(&pwfile, format!("{}\n", password)).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&pwfile, std::fs::Permissions::from_mode(0o600));
        }
        let args = db_runtime::postgres_init_args(&home, &share, &username, &pwfile);
        let result = run_command(&db_runtime::postgres_initdb_path(&home), &args, None, Duration::from_secs(120)).await;
        let _ = std::fs::remove_file(&pwfile);
        let (ok, out) = result?;
        if !ok {
            return Err(format!("initdb gagal: {}", out));
        }
    }
    std::fs::write(
        db_runtime::postgres_conf_path(&home),
        db_runtime::generate_postgresql_conf(port as u16, &home),
    )
    .map_err(|e| e.to_string())?;
    if !port_free(port as u16) {
        return Err(format!("Port {} sudah digunakan", port));
    }
    spawn_service(
        state,
        &id,
        db_runtime::postgres_binary_path(&home),
        db_runtime::postgres_start_args(&home),
    )
    .await?;
    if !wait_pg_ready(port, &username, &password).await {
        stop_service_process(state, &id).await;
        return Err("PostgreSQL tidak merespons health check dalam 30 detik".to_string());
    }
    // Pastikan database tujuan ada.
    if database_name != "postgres" {
        let pool = pg_connect("127.0.0.1", port, &username, &password, "postgres").await?;
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
            .bind(&database_name)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            sqlx::query(&format!("CREATE DATABASE \"{}\"", database_name.replace('"', "\"\"")))
                .execute(&pool)
                .await
                .map_err(|e| e.to_string())?;
        }
        pool.close().await;
    }
    Ok(())
}

async fn start_mariadb(state: &AppState, row: &sqlx::sqlite::SqliteRow) -> Result<(), String> {
    let id: String = row.get("id");
    let port: i64 = row.try_get("port").ok().flatten().unwrap_or(3306);
    let username: String = row.try_get("username").ok().flatten().unwrap_or_else(|| "root".to_string());
    let database_name: String = row.try_get("database_name").ok().flatten().unwrap_or_else(|| "webforge".to_string());
    let password = decrypted_password(state, row).ok_or("Password database lokal tidak dapat didekripsi")?;
    let home = db_runtime::mariadb_home(&state.config.data_dir, &id);
    let native_dir = state
        .native_lib_dir
        .clone()
        .ok_or("nativeLibraryDir tidak diketahui")?;
    for dir in db_runtime::mariadb_required_dirs(&home) {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    let share = ensure_share(state, "mariadb", &home)?;
    std::fs::write(
        db_runtime::mariadb_cnf_path(&home),
        db_runtime::generate_mariadb_cnf(port as u16, &home, &share),
    )
    .map_err(|e| e.to_string())?;
    let binary = db_runtime::mariadb_binary_path(&native_dir);
    let data_dir = db_runtime::mariadb_data_dir(&home);
    if !data_dir.join("mysql").is_dir() {
        let mut sql = String::new();
        for f in db_runtime::mariadb_bootstrap_sql_files(&share) {
            let content = std::fs::read_to_string(&f)
                .map_err(|e| format!("File bootstrap {} tidak terbaca: {}", f.display(), e))?;
            sql.push_str(&content);
            sql.push('\n');
        }
        sql.push_str(&db_runtime::mariadb_set_root_password_sql(&password));
        let (ok, out) = run_command(
            &binary,
            &db_runtime::mariadb_bootstrap_args(&home),
            Some(&sql),
            Duration::from_secs(180),
        )
        .await?;
        if !ok {
            return Err(format!("Bootstrap MariaDB gagal: {}", out));
        }
    }
    if !port_free(port as u16) {
        return Err(format!("Port {} sudah digunakan", port));
    }
    spawn_service(state, &id, binary, db_runtime::mariadb_start_args(&home)).await?;
    if !wait_mysql_ready(port, &username, &password).await {
        stop_service_process(state, &id).await;
        return Err("MariaDB tidak merespons health check dalam 30 detik".to_string());
    }
    let pool = mysql_connect("127.0.0.1", port, &username, &password, "mysql").await?;
    sqlx::query(&format!(
        "CREATE DATABASE IF NOT EXISTS `{}`",
        database_name.replace('`', "``")
    ))
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    pool.close().await;
    Ok(())
}

async fn stop_service_process(state: &AppState, id: &str) {
    let handle = state.processes.lock().await.remove(&format!("db:{}", id));
    if let Some(ProcessHandle::Child(arc)) = handle {
        let mut guard = arc.lock().await;
        if let Some(child) = guard.as_mut() {
            #[cfg(unix)]
            {
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
                }
            }
            let _ = child.kill().await;
        }
    }
}

pub async fn start_database(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let kind = kind_of(&row);
    if kind != "postgresql" && kind != "mariadb" {
        return Err(ApiError::bad_request(format!("Database {} tidak memiliki layanan lokal untuk dijalankan", kind)));
    }
    let status: String = row.try_get("status").unwrap_or_else(|_| "stopped".to_string());
    if status == "running" {
        return Ok(Json(db_out(&row)));
    }
    set_db_status(&state, &id, "starting", None).await;
    let result = if kind == "postgresql" {
        start_postgres(&state, &row).await
    } else {
        start_mariadb(&state, &row).await
    };
    match result {
        Ok(()) => {
            set_db_status(&state, &id, "running", None).await;
            crate::state::app_log(&state.pool, "database", "info", &format!("{} berjalan (health check lolos)", kind), None).await;
        }
        Err(e) => {
            set_db_status(&state, &id, "error", Some(&e)).await;
            crate::state::app_log(&state.pool, "database", "error", &e, None).await;
        }
    }
    let row = db_row(&state, &id).await?;
    Ok(Json(db_out(&row)))
}

pub async fn stop_database(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let kind = kind_of(&row);
    if kind == "postgresql" {
        // pg_ctl stop bersih lebih dulu, lalu pastikan proses mati.
        let home = db_runtime::postgres_home(&state.config.data_dir, &id);
        let pg_ctl = db_runtime::postgres_pg_ctl_path(&home);
        if pg_ctl.exists() {
            let _ = run_command(&pg_ctl, &db_runtime::postgres_stop_args(&home), None, Duration::from_secs(35)).await;
        }
    }
    stop_service_process(&state, &id).await;
    set_db_status(&state, &id, "stopped", None).await;
    let row = db_row(&state, &id).await?;
    Ok(Json(db_out(&row)))
}

pub async fn delete_database(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let kind = kind_of(&row);
    if kind == "postgresql" || kind == "mariadb" {
        stop_service_process(&state, &id).await;
    }
    let path: String = row.get("path");
    sqlx::query("DELETE FROM database_instances WHERE id=?1")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(db_err)?;
    if kind == "sqlite" {
        let p = PathBuf::from(&path);
        if is_inside(&state.config.databases_dir(), &p) && p.exists() {
            let _ = std::fs::remove_file(&p);
            let _ = std::fs::remove_file(format!("{}-wal", path));
            let _ = std::fs::remove_file(format!("{}-shm", path));
        }
    } else if kind == "postgresql" || kind == "mariadb" {
        let home = PathBuf::from(&path);
        let services_root = state.config.data_dir.join("databases-services");
        if is_inside(&services_root, &home) && home.exists() {
            let _ = std::fs::remove_dir_all(&home);
        }
    }
    Ok(Json(json!({"ok": true})))
}

async fn pg_pool_for(state: &AppState, row: &sqlx::sqlite::SqliteRow) -> ApiResult<sqlx::PgPool> {
    let host: String = row.try_get("host").ok().flatten().unwrap_or_else(|| "127.0.0.1".to_string());
    let port: i64 = row.try_get("port").ok().flatten().unwrap_or(5432);
    let username: String = row.try_get("username").ok().flatten().unwrap_or_else(|| "postgres".to_string());
    let database: String = row.try_get("database_name").ok().flatten().unwrap_or_else(|| "postgres".to_string());
    let password = decrypted_password(state, row).unwrap_or_default();
    pg_connect(&host, port, &username, &password, &database)
        .await
        .map_err(ApiError::bad_request)
}

async fn mysql_pool_for(state: &AppState, row: &sqlx::sqlite::SqliteRow) -> ApiResult<sqlx::MySqlPool> {
    let host: String = row.try_get("host").ok().flatten().unwrap_or_else(|| "127.0.0.1".to_string());
    let port: i64 = row.try_get("port").ok().flatten().unwrap_or(3306);
    let username: String = row.try_get("username").ok().flatten().unwrap_or_else(|| "root".to_string());
    let database: String = row.try_get("database_name").ok().flatten().unwrap_or_else(|| "mysql".to_string());
    let password = decrypted_password(state, row).unwrap_or_default();
    mysql_connect(&host, port, &username, &password, &database)
        .await
        .map_err(ApiError::bad_request)
}

pub async fn list_tables(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let kind = kind_of(&row);
    match kind.as_str() {
        "sqlite" => {
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
        "postgresql" | "external-postgresql" => {
            let pool = pg_pool_for(&state, &row).await?;
            let tables = sqlx::query(
                "SELECT table_name FROM information_schema.tables WHERE table_schema='public' ORDER BY table_name",
            )
            .fetch_all(&pool)
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
            let mut out = Vec::new();
            for t in &tables {
                let tname: String = t.get("table_name");
                let cols = sqlx::query(
                    "SELECT column_name, data_type FROM information_schema.columns WHERE table_schema='public' AND table_name=$1 ORDER BY ordinal_position",
                )
                .bind(&tname)
                .fetch_all(&pool)
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
                let columns: Vec<Value> = cols
                    .iter()
                    .map(|c| json!({"name": c.get::<String, _>("column_name"), "type": c.get::<String, _>("data_type"), "pk": false}))
                    .collect();
                out.push(json!({"name": tname, "columns": columns}));
            }
            pool.close().await;
            Ok(Json(json!(out)))
        }
        "mariadb" | "external-mysql" => {
            let pool = mysql_pool_for(&state, &row).await?;
            let database: String = row.try_get("database_name").ok().flatten().unwrap_or_default();
            let tables = sqlx::query(
                "SELECT TABLE_NAME AS table_name FROM information_schema.tables WHERE table_schema=? ORDER BY TABLE_NAME",
            )
            .bind(&database)
            .fetch_all(&pool)
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
            let mut out = Vec::new();
            for t in &tables {
                let tname: String = t.get("table_name");
                let cols = sqlx::query(
                    "SELECT COLUMN_NAME AS column_name, DATA_TYPE AS data_type, COLUMN_KEY AS column_key FROM information_schema.columns WHERE table_schema=? AND table_name=? ORDER BY ORDINAL_POSITION",
                )
                .bind(&database)
                .bind(&tname)
                .fetch_all(&pool)
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
                let columns: Vec<Value> = cols
                    .iter()
                    .map(|c| json!({
                        "name": c.get::<String, _>("column_name"),
                        "type": c.get::<String, _>("data_type"),
                        "pk": c.get::<String, _>("column_key") == "PRI",
                    }))
                    .collect();
                out.push(json!({"name": tname, "columns": columns}));
            }
            pool.close().await;
            Ok(Json(json!(out)))
        }
        other => Err(ApiError::bad_request(format!("Kind {} tidak didukung", other))),
    }
}

#[derive(Deserialize)]
pub struct QueryBody {
    pub sql: String,
}

fn sqlite_value_of(row: &sqlx::sqlite::SqliteRow, idx: usize) -> Value {
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

fn pg_value_of(row: &sqlx::postgres::PgRow, idx: usize) -> Value {
    if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
        return v.map(|s| json!(s)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        return v.map(|n| json!(n)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(idx) {
        return v.map(|n| json!(n)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return v.map(|n| json!(n)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(idx) {
        return v.map(|n| json!(n)).unwrap_or(Value::Null);
    }
    Value::Null
}

fn mysql_value_of(row: &sqlx::mysql::MySqlRow, idx: usize) -> Value {
    if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
        return v.map(|s| json!(s)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        return v.map(|n| json!(n)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return v.map(|n| json!(n)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(idx) {
        return v
            .map(|b| json!(String::from_utf8_lossy(&b).to_string()))
            .unwrap_or(Value::Null);
    }
    Value::Null
}

fn is_select_sql(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    upper.starts_with("SELECT") || upper.starts_with("PRAGMA") || upper.starts_with("WITH") || upper.starts_with("EXPLAIN") || upper.starts_with("SHOW") || upper.starts_with("DESCRIBE")
}

pub async fn run_query(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<QueryBody>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    let sql = b.sql.trim().to_string();
    if sql.is_empty() {
        return Err(ApiError::bad_request("SQL wajib diisi"));
    }
    let kind = kind_of(&row);
    let is_select = is_select_sql(&sql);
    match kind.as_str() {
        "sqlite" => {
            let path: String = row.get("path");
            let pool = open_db(&path).await?;
            let result = if is_select {
                match sqlx::query(&sql).fetch_all(&pool).await {
                    Ok(rows) => {
                        let columns: Vec<String> = rows
                            .first()
                            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                            .unwrap_or_default();
                        let out_rows: Vec<Value> = rows
                            .iter()
                            .map(|r| json!(r.columns().iter().enumerate().map(|(i, _)| sqlite_value_of(r, i)).collect::<Vec<_>>()))
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
        "postgresql" | "external-postgresql" => {
            let pool = pg_pool_for(&state, &row).await?;
            let result = if is_select {
                match sqlx::query(&sql).fetch_all(&pool).await {
                    Ok(rows) => {
                        let columns: Vec<String> = rows
                            .first()
                            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                            .unwrap_or_default();
                        let out_rows: Vec<Value> = rows
                            .iter()
                            .map(|r| json!(r.columns().iter().enumerate().map(|(i, _)| pg_value_of(r, i)).collect::<Vec<_>>()))
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
        "mariadb" | "external-mysql" => {
            let pool = mysql_pool_for(&state, &row).await?;
            let result = if is_select {
                match sqlx::query(&sql).fetch_all(&pool).await {
                    Ok(rows) => {
                        let columns: Vec<String> = rows
                            .first()
                            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                            .unwrap_or_default();
                        let out_rows: Vec<Value> = rows
                            .iter()
                            .map(|r| json!(r.columns().iter().enumerate().map(|(i, _)| mysql_value_of(r, i)).collect::<Vec<_>>()))
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
        other => Err(ApiError::bad_request(format!("Kind {} tidak didukung", other))),
    }
}

pub async fn backup_database(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let row = db_row(&state, &id).await?;
    if kind_of(&row) != "sqlite" {
        return Err(ApiError::bad_request("Backup hanya tersedia untuk SQLite saat ini"));
    }
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
    if kind_of(&row) != "sqlite" {
        return Err(ApiError::bad_request("Restore hanya tersedia untuk SQLite saat ini"));
    }
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
