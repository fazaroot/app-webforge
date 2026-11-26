//! Self-test engine nyata untuk `POST /api/diagnostics/self-test`.
//!
//! Setiap engine diuji end-to-end di direktori temp
//! `<data_dir>/selftest/<timestamp>/<engine>`: server benar-benar
//! dimulai di port ephemeral, di-GET lewat HTTP, marker unik di
//! respons diverifikasi, lalu proses dihentikan dan direktori
//! dibersihkan. PostgreSQL/MariaDB memakai instance temp terpisah
//! (initdb/bootstrap temp + `SELECT 1` nyata) dan tidak menyentuh
//! instance/data pengguna. Engine yang runtime-nya tidak tersedia
//! dilaporkan `ok:false` dengan alasan probe-nya.
//!
//! Konfigurasi dan argumen memakai ulang logika yang sudah ada
//! (`webservers.rs`, `php.rs`, `python_runtime.rs`, `db_runtime.rs`);
//! modul ini hanya mengorkestrasi spawn + verifikasi.

use crate::state::AppState;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
pub struct SelfTestResult {
    pub engine: String,
    pub ok: bool,
    pub duration_ms: u64,
    pub detail: String,
    pub error: Option<String>,
    pub lan_ok: Option<bool>,
    pub lan_detail: Option<String>,
    pub lan_error: Option<String>,
}

/// Urutan engine self-test (id runtime yang sama dengan /api/runtimes).
pub const SELFTEST_ENGINES: [&str; 9] = [
    "static", "php", "caddy", "nginx", "apache", "python", "node", "postgresql", "mariadb",
];

fn ok_result(engine: &str, duration_ms: u64, detail: String) -> SelfTestResult {
    SelfTestResult {
        engine: engine.to_string(),
        ok: true,
        duration_ms,
        detail,
        error: None,
        lan_ok: None,
        lan_detail: None,
        lan_error: None,
    }
}

fn fail_result(engine: &str, duration_ms: u64, detail: String, error: String) -> SelfTestResult {
    SelfTestResult {
        engine: engine.to_string(),
        ok: false,
        duration_ms,
        detail,
        error: Some(error),
        lan_ok: None,
        lan_detail: None,
        lan_error: None,
    }
}

struct EngineOutcome {
    detail: String,
    error: Option<String>,
    lan_ok: Option<bool>,
    lan_detail: Option<String>,
    lan_error: Option<String>,
}

impl EngineOutcome {
    fn ok(detail: String) -> Self {
        Self {
            detail,
            error: None,
            lan_ok: None,
            lan_detail: None,
            lan_error: None,
        }
    }

    fn err(error: String) -> Self {
        Self {
            detail: String::new(),
            error: Some(error),
            lan_ok: None,
            lan_detail: None,
            lan_error: None,
        }
    }
}

pub fn selftest_marker(engine: &str) -> String {
    format!("WEBFORGE-SELFTEST-{}-{}", engine, crate::util::new_id())
}

pub fn selftest_index_html(marker: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html><head><title>WebForge Self-Test</title></head><body>{}</body></html>\n",
        marker
    )
}

/// Skrip server Python minimal: melayani marker di semua path.
pub fn selftest_python_script(marker: &str) -> String {
    format!(
        "import http.server, os\n\
         MARKER = {:?}\n\
         class Handler(http.server.BaseHTTPRequestHandler):\n\
         \x20   def do_GET(self):\n\
         \x20       body = MARKER.encode()\n\
         \x20       self.send_response(200)\n\
         \x20       self.send_header('Content-Type', 'text/html')\n\
         \x20       self.send_header('Content-Length', str(len(body)))\n\
         \x20       self.end_headers()\n\
         \x20       self.wfile.write(body)\n\
         \x20   def log_message(self, *args):\n\
         \x20       pass\n\
         http.server.HTTPServer((os.environ.get('HOST', '127.0.0.1'), int(os.environ.get('PORT', '0'))), Handler).serve_forever()\n",
        marker
    )
}

/// Skrip server Node minimal. Port ditulis literal karena NodeService
/// Android tidak meneruskan environment ke proses :node.
pub fn selftest_node_script(marker: &str, port: u16, host: &str) -> String {
    format!(
        "const http = require('http');\n\
         const marker = {:?};\n\
         http.createServer((req, res) => {{\n\
         \x20 res.writeHead(200, {{ 'Content-Type': 'text/html' }});\n\
         \x20 res.end(marker);\n\
         }}).listen({}, {:?});\n",
        marker, port, host
    )
}

/// Spesifikasi peluncuran proses self-test satu engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfTestLaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub envs: Vec<(String, String)>,
}

pub fn php_launch(binary: &Path, host: &str, port: u16, docroot: &Path, ini_path: &Path) -> SelfTestLaunch {
    SelfTestLaunch {
        program: binary.to_path_buf(),
        args: crate::php::build_php_server_args(host, port, docroot, ini_path),
        cwd: docroot.to_path_buf(),
        envs: vec![(
            "PHPRC".to_string(),
            ini_path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        )],
    }
}

pub fn webserver_launch(engine: &str, binary: &Path, config_path: &Path, home: &Path) -> SelfTestLaunch {
    let mut envs = Vec::new();
    if engine == "caddy" {
        envs.push((
            "XDG_DATA_HOME".to_string(),
            home.join("data").to_string_lossy().to_string(),
        ));
        envs.push((
            "XDG_CONFIG_HOME".to_string(),
            home.join("config").to_string_lossy().to_string(),
        ));
    }
    SelfTestLaunch {
        program: binary.to_path_buf(),
        args: crate::webservers::launch_args(engine, config_path, home),
        cwd: home.to_path_buf(),
        envs,
    }
}

pub fn python_launch(binary: &Path, script_path: &Path, host: &str, port: u16, extra_envs: Vec<(String, String)>) -> SelfTestLaunch {
    let mut envs = extra_envs;
    envs.push(("HOST".to_string(), host.to_string()));
    envs.push(("PORT".to_string(), port.to_string()));
    SelfTestLaunch {
        program: binary.to_path_buf(),
        args: vec![script_path.to_string_lossy().to_string()],
        cwd: script_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
        envs,
    }
}

pub fn node_launch(binary: &Path, script_path: &Path, host: &str, port: u16) -> SelfTestLaunch {
    SelfTestLaunch {
        program: binary.to_path_buf(),
        args: vec![script_path.to_string_lossy().to_string()],
        cwd: script_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
        envs: vec![
            ("HOST".to_string(), host.to_string()),
            ("PORT".to_string(), port.to_string()),
        ],
    }
}

fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("Gagal mengalokasikan port ephemeral: {}", e))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

async fn http_get(host: &str, port: u16) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = format!("{}:{}", host, port);
    let mut stream = match tokio::time::timeout(
        Duration::from_millis(800),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(s)) => s,
        _ => return None,
    };
    let request = format!(
        "GET / HTTP/1.0\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        host, port
    );
    if stream.write_all(request.as_bytes()).await.is_err() {
        return None;
    }
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_millis(800), stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 65536 {
                    break;
                }
            }
            _ => break,
        }
    }
    if buf.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&buf).to_string())
    }
}

fn engine_timeout(engine: &str) -> Duration {
    match engine {
        "python" | "node" => Duration::from_secs(30),
        "php" | "caddy" | "nginx" | "apache" => Duration::from_secs(15),
        _ => Duration::from_secs(10),
    }
}

/// Proses anak self-test dengan buffer ekor output (stdout+stderr).
struct SelfTestChild {
    child: tokio::process::Child,
    output_tail: Arc<Mutex<VecDeque<String>>>,
}

fn spawn_selftest_child(
    state: &AppState,
    launch: &SelfTestLaunch,
    home: &Path,
) -> Result<SelfTestChild, String> {
    let mut cmd = tokio::process::Command::new(&launch.program);
    cmd.args(&launch.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(false);
    crate::util::prepare_spawn(
        &mut cmd,
        state.native_lib_dir.as_deref(),
        home,
        Some(&launch.cwd),
    );
    for (k, v) in &launch.envs {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Gagal menjalankan {}: {}", launch.program.display(), e))?;
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    for reader in [child.stdout.take().map(|r| Box::new(r) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|r| Box::new(r) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)] {
        if let Some(reader) = reader {
            let tail = tail.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(reader).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        let mut guard = tail.lock().await;
                        crate::process::push_capped_line(&mut guard, line);
                    }
                }
            });
        }
    }
    Ok(SelfTestChild { child, output_tail: tail })
}

async fn tail_text(tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    let guard = tail.lock().await;
    guard.iter().cloned().collect::<Vec<_>>().join("\n")
}

/// Tunggu marker muncul di respons HTTP; proses yang keluar lebih dulu
/// atau timeout menghasilkan error dengan ekor output proses.
async fn wait_marker(
    host: &str,
    port: u16,
    marker: &str,
    timeout: Duration,
    child: Option<&mut SelfTestChild>,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut child = child;
    loop {
        if let Some(c) = child.as_deref_mut() {
            if let Ok(Some(status)) = c.child.try_wait() {
                // Beri kesempatan reader menguras pipe.
                tokio::time::sleep(Duration::from_millis(150)).await;
                let tail = tail_text(&c.output_tail).await;
                return Err(format!(
                    "Proses keluar sebelum marker terverifikasi (exit code {}). Output terakhir:\n{}",
                    status.code().map(|c| c.to_string()).unwrap_or_else(|| "tidak diketahui".to_string()),
                    tail
                ));
            }
        }
        if let Some(body) = http_get(host, port).await {
            if body.contains(marker) {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let tail = match child.as_deref() {
                Some(c) => tail_text(&c.output_tail).await,
                None => String::new(),
            };
            return Err(format!(
                "Marker tidak ditemukan dalam respons HTTP setelah {} detik. Output terakhir:\n{}",
                timeout.as_secs(),
                tail
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn run_capture(
    state: &AppState,
    program: &Path,
    args: &[String],
    cwd: &Path,
    home: &Path,
    stdin_data: Option<&str>,
    timeout: Duration,
) -> Result<(bool, String), String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if stdin_data.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    crate::util::prepare_spawn(&mut cmd, state.native_lib_dir.as_deref(), home, Some(cwd));
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
        .map_err(|_| format!("{} timeout setelah {} detik", program.display(), timeout.as_secs()))?
        .map_err(|e| e.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text.trim().to_string()))
}

fn tail_of(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        text.to_string()
    } else {
        chars[chars.len() - max_chars..].iter().collect()
    }
}

struct EnginePaths {
    base: PathBuf,
    dir: PathBuf,
}

async fn run_static(
    state: &AppState,
    paths: &EnginePaths,
    marker: &str,
    bind_host: &str,
    probe_host: &str,
) -> Result<String, String> {
    let _ = state;
    let listener = tokio::net::TcpListener::bind((bind_host, 0))
        .await
        .map_err(|e| format!("Gagal bind port ephemeral: {}", e))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let app = crate::static_app(paths.dir.clone());
    let serve = std::future::IntoFuture::into_future(axum::serve(listener, app));
    let handle = tokio::spawn(serve);
    let result = wait_marker(probe_host, port, marker, engine_timeout("static"), None).await;
    handle.abort();
    result.map(|_| {
        format!(
            "Marker terverifikasi via HTTP GET / di {}:{}",
            probe_host, port
        )
    })
}

async fn run_http_engine(
    state: &AppState,
    engine: &str,
    paths: &EnginePaths,
    marker: &str,
    bind_host: &str,
    probe_host: &str,
) -> Result<String, String> {
    let port = free_port()?;
    if engine == "node" {
        // Node Android tidak punya binary PATH; ia berjalan lewat
        // NodeService, jadi tangani sebelum pencarian binary.
        let script = paths.dir.join("server.js");
        std::fs::write(&script, selftest_node_script(marker, port, bind_host))
            .map_err(|e| format!("Gagal menulis skrip Node self-test: {}", e))?;
        return run_node_selftest(state, paths, &script, bind_host, probe_host, port, marker).await;
    }
    let binary = crate::process::find_engine_binary(state, engine)
        .ok_or_else(|| format!("Binary engine {} tidak ditemukan", engine))?;
    let (launch, home) = match engine {
        "php" => {
            let settings = crate::php::load_php_settings(&state.pool).await;
            let ini = crate::php::write_php_ini(&paths.dir, &settings)
                .map_err(|e| format!("Gagal menulis php.ini self-test: {}", e))?;
            (
                php_launch(&binary, bind_host, port, &paths.dir, &ini),
                paths.dir.clone(),
            )
        }
        "caddy" | "nginx" | "apache" => {
            let input = crate::webservers::WebServerInput {
                host: bind_host.to_string(),
                port,
                doc_root: paths.dir.clone(),
                project_path: paths.dir.clone(),
                data_dir: paths.base.clone(),
                server_id: format!("selftest-{}", engine),
                proxy_port: None,
                php_fpm_port: None,
                custom_config: None,
            };
            let config_path = crate::webservers::write_config(&input, engine)
                .map_err(|e| format!("Gagal menulis konfigurasi {}: {}", engine, e))?;
            let home = crate::webservers::server_home(&paths.base, &format!("selftest-{}", engine), engine);
            let validate_args = crate::webservers::validate_args(engine, &config_path, &home);
            let (valid, output) = run_capture(
                state,
                &binary,
                &validate_args,
                &home,
                &home,
                None,
                Duration::from_secs(10),
            )
            .await?;
            if !valid {
                return Err(format!(
                    "Validasi konfigurasi {} gagal: {}",
                    engine,
                    tail_of(&output, 1000)
                ));
            }
            (webserver_launch(engine, &binary, &config_path, &home), home)
        }
        "python" => {
            let script = paths.dir.join("server.py");
            std::fs::write(&script, selftest_python_script(marker))
                .map_err(|e| format!("Gagal menulis skrip Python self-test: {}", e))?;
            let extra = if crate::python_runtime::is_bundled_python(&binary, state.native_lib_dir.as_deref()) {
                crate::python_runtime::python_env(&state.config.data_dir, "selftest")
            } else {
                Vec::new()
            };
            (
                python_launch(&binary, &script, bind_host, port, extra),
                paths.dir.clone(),
            )
        }
        other => return Err(format!("Engine {} tidak dikenal untuk self-test HTTP", other)),
    };
    let mut child = spawn_selftest_child(state, &launch, &home)?;
    let result = wait_marker(
        probe_host,
        port,
        marker,
        engine_timeout(engine),
        Some(&mut child),
    )
    .await;
    let _ = child.child.kill().await;
    let _ = child.child.wait().await;
    result.map(|_| {
        format!(
            "Marker terverifikasi via HTTP GET / di {}:{}",
            probe_host, port
        )
    })
}

async fn run_node_selftest(
    state: &AppState,
    paths: &EnginePaths,
    script: &Path,
    bind_host: &str,
    probe_host: &str,
    port: u16,
    marker: &str,
) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        let node_busy = state
            .processes
            .lock()
            .await
            .values()
            .any(|h| matches!(h, crate::state::ProcessHandle::Node(_)));
        if node_busy {
            return Err(
                "NodeService sedang menjalankan server Node aktif; self-test Node dilewati agar server tidak terganggu"
                    .to_string(),
            );
        }
        let log_path = paths.dir.join("node.log");
        let status_path = paths.dir.join("node-status.txt");
        let args = vec![script.to_string_lossy().to_string()];
        let started = crate::jni_bridge::node_start(
            &args,
            &paths.dir.to_string_lossy(),
            &log_path.to_string_lossy(),
            &status_path.to_string_lossy(),
        );
        if !started {
            return Err("NodeService gagal dimulai lewat JNI untuk self-test Node".to_string());
        }
        let deadline = tokio::time::Instant::now() + engine_timeout("node");
        let result = loop {
            if let Some(body) = http_get(probe_host, port).await {
                if body.contains(marker) {
                    break Ok(format!(
                        "Marker terverifikasi via HTTP GET / di {}:{}",
                        probe_host, port
                    ));
                }
            }
            let status = std::fs::read_to_string(&status_path).unwrap_or_default();
            if status.trim().parse::<i32>().is_ok() {
                let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                break Err(format!(
                    "Node keluar sebelum marker terverifikasi (exit code {}). Log:\n{}",
                    status.trim(),
                    tail_of(&log, 1000)
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                break Err(format!(
                    "Marker tidak ditemukan dalam respons HTTP setelah {} detik. Log:\n{}",
                    engine_timeout("node").as_secs(),
                    tail_of(&log, 1000)
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        let _ = crate::jni_bridge::node_stop();
        result
    }
    #[cfg(not(target_os = "android"))]
    {
        let binary = crate::process::find_engine_binary(state, "node")
            .ok_or_else(|| "Binary Node tidak ditemukan".to_string())?;
        let launch = node_launch(&binary, script, bind_host, port);
        let mut child = spawn_selftest_child(state, &launch, &paths.dir)?;
        let result = wait_marker(
            probe_host,
            port,
            marker,
            engine_timeout("node"),
            Some(&mut child),
        )
        .await;
        let _ = child.child.kill().await;
        let _ = child.child.wait().await;
        result.map(|_| {
            format!(
                "Marker terverifikasi via HTTP GET / di {}:{}",
                probe_host, port
            )
        })
    }
}

async fn pg_select_one(port: u16, username: &str, password: &str) -> Result<(), String> {
    let opts = sqlx::postgres::PgConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username(username)
        .password(password)
        .database("postgres");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with(opts)
        .await
        .map_err(|e| e.to_string())?;
    let result = sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string());
    pool.close().await;
    result
}

async fn mysql_select_one(port: u16, username: &str, password: &str) -> Result<(), String> {
    let opts = sqlx::mysql::MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username(username)
        .password(password)
        .database("mysql");
    let pool = sqlx::mysql::MySqlPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with(opts)
        .await
        .map_err(|e| e.to_string())?;
    let result = sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string());
    pool.close().await;
    result
}

async fn run_postgres_selftest(state: &AppState, paths: &EnginePaths) -> Result<String, String> {
    use crate::db_runtime;
    let home = paths.dir.clone();
    for dir in db_runtime::postgres_required_dirs(&home) {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    let bundled = state.native_lib_dir.as_deref().and_then(|d| {
        let pg = crate::runtime::bundled_binary(Some(d), "libpostgres.so")?;
        let _ = crate::runtime::bundled_binary(Some(d), "libinitdb.so")?;
        let _ = crate::runtime::bundled_binary(Some(d), "libpgctl.so")?;
        Some((d.to_path_buf(), pg))
    });
    let (initdb, postgres, pg_ctl, share) = if let Some((native_dir, _)) = bundled {
        db_runtime::ensure_postgres_symlinks(&home, &native_dir)?;
        let share = crate::routes::databases::ensure_share(state, "postgresql", &home)?;
        (
            db_runtime::postgres_initdb_path(&home),
            db_runtime::postgres_binary_path(&home),
            Some(db_runtime::postgres_pg_ctl_path(&home)),
            share,
        )
    } else {
        let initdb = crate::runtime::find_in_path("initdb")
            .ok_or_else(|| "initdb tidak ditemukan untuk self-test PostgreSQL".to_string())?;
        let postgres = crate::runtime::find_in_path("postgres")
            .ok_or_else(|| "postgres tidak ditemukan untuk self-test PostgreSQL".to_string())?;
        let pg_ctl = crate::runtime::find_in_path("pg_ctl");
        let share = match crate::runtime::find_in_path("pg_config") {
            Some(pg_config) => {
                let (ok, out) = run_capture(
                    state,
                    &pg_config,
                    &["--sharedir".to_string()],
                    &home,
                    &home,
                    None,
                    Duration::from_secs(10),
                )
                .await?;
                if !ok || out.is_empty() {
                    return Err(format!("pg_config --sharedir gagal: {}", tail_of(&out, 500)));
                }
                PathBuf::from(out.lines().next().unwrap_or("").trim())
            }
            None => {
                return Err(
                    "pg_config tidak ditemukan; share PostgreSQL host tidak diketahui".to_string(),
                )
            }
        };
        (initdb, postgres, pg_ctl, share)
    };

    let port = free_port()?;
    let username = "webforge";
    let password = crate::secrets::generate_password();
    let pwfile = home.join("pwfile");
    std::fs::write(&pwfile, format!("{}\n", password)).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&pwfile, std::fs::Permissions::from_mode(0o600));
    }
    let init_args = db_runtime::postgres_init_args(&home, &share, username, &pwfile);
    let init_result = run_capture(
        state,
        &initdb,
        &init_args,
        &home,
        &home,
        None,
        Duration::from_secs(120),
    )
    .await;
    let _ = std::fs::remove_file(&pwfile);
    let (ok, out) = init_result?;
    if !ok {
        return Err(format!("initdb self-test gagal: {}", tail_of(&out, 1500)));
    }
    std::fs::write(
        db_runtime::postgres_conf_path(&home),
        db_runtime::generate_postgresql_conf(port, &home),
    )
    .map_err(|e| e.to_string())?;

    let launch = SelfTestLaunch {
        program: postgres,
        args: db_runtime::postgres_start_args(&home),
        cwd: home.clone(),
        envs: Vec::new(),
    };
    let mut child = spawn_selftest_child(state, &launch, &home)?;
    let mut last_err = String::new();
    let mut ready = false;
    for _ in 0..60 {
        if let Ok(Some(status)) = child.child.try_wait() {
            let tail = tail_text(&child.output_tail).await;
            return Err(format!(
                "PostgreSQL self-test keluar sebelum siap (exit code {}). Output:\n{}",
                status.code().map(|c| c.to_string()).unwrap_or_else(|| "tidak diketahui".to_string()),
                tail
            ));
        }
        match pg_select_one(port, username, &password).await {
            Ok(()) => {
                ready = true;
                break;
            }
            Err(e) => last_err = e,
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if let Some(pg_ctl) = pg_ctl {
        let _ = run_capture(
            state,
            &pg_ctl,
            &db_runtime::postgres_stop_args(&home),
            &home,
            &home,
            None,
            Duration::from_secs(35),
        )
        .await;
    }
    let _ = child.child.kill().await;
    let _ = child.child.wait().await;
    if ready {
        Ok(format!(
            "Instance temp PostgreSQL merespons SELECT 1 di port {}",
            port
        ))
    } else {
        let tail = tail_text(&child.output_tail).await;
        Err(format!(
            "PostgreSQL self-test tidak merespons SELECT 1 dalam 30 detik: {}. Output:\n{}",
            last_err, tail
        ))
    }
}

async fn run_mariadb_selftest(state: &AppState, paths: &EnginePaths) -> Result<String, String> {
    use crate::db_runtime;
    let home = paths.dir.clone();
    for dir in db_runtime::mariadb_required_dirs(&home) {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    let (binary, share) = if let Some(native_dir) = state.native_lib_dir.as_deref() {
        let binary = db_runtime::mariadb_binary_path(native_dir);
        if binary.is_file() {
            let share = crate::routes::databases::ensure_share(state, "mariadb", &home)?;
            (binary, share)
        } else {
            host_mariadb_binary_share()?
        }
    } else {
        host_mariadb_binary_share()?
    };

    let port = free_port()?;
    let password = crate::secrets::generate_password();
    std::fs::write(
        db_runtime::mariadb_cnf_path(&home),
        db_runtime::generate_mariadb_cnf(port, &home, &share),
    )
    .map_err(|e| e.to_string())?;

    let mut contents = Vec::new();
    for f in db_runtime::mariadb_bootstrap_sql_files(&share) {
        let content = std::fs::read_to_string(&f)
            .map_err(|e| format!("File bootstrap {} tidak terbaca: {}", f.display(), e))?;
        contents.push(content);
    }
    let sql = db_runtime::assemble_mariadb_bootstrap_sql(&contents, &password);
    let (ok, out) = run_capture(
        state,
        &binary,
        &db_runtime::mariadb_bootstrap_args(&home),
        &home,
        &home,
        Some(&sql),
        Duration::from_secs(180),
    )
    .await?;
    if !ok {
        let log = std::fs::read_to_string(db_runtime::mariadb_error_log_path(&home)).unwrap_or_default();
        return Err(format!(
            "Bootstrap MariaDB self-test gagal: {}. Log:\n{}",
            tail_of(&out, 1000),
            tail_of(&log, 1000)
        ));
    }

    let launch = SelfTestLaunch {
        program: binary,
        args: db_runtime::mariadb_start_args(&home),
        cwd: home.clone(),
        envs: Vec::new(),
    };
    let mut child = spawn_selftest_child(state, &launch, &home)?;
    let mut last_err = String::new();
    let mut ready = false;
    for _ in 0..60 {
        if let Ok(Some(status)) = child.child.try_wait() {
            let log = std::fs::read_to_string(db_runtime::mariadb_error_log_path(&home)).unwrap_or_default();
            return Err(format!(
                "MariaDB self-test keluar sebelum siap (exit code {}). Log:\n{}",
                status.code().map(|c| c.to_string()).unwrap_or_else(|| "tidak diketahui".to_string()),
                tail_of(&log, 1500)
            ));
        }
        match mysql_select_one(port, "root", &password).await {
            Ok(()) => {
                ready = true;
                break;
            }
            Err(e) => last_err = e,
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    #[cfg(unix)]
    {
        if let Some(pid) = child.child.id() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            let _ = tokio::time::timeout(Duration::from_secs(10), child.child.wait()).await;
        }
    }
    let _ = child.child.kill().await;
    let _ = child.child.wait().await;
    if ready {
        Ok(format!(
            "Instance temp MariaDB merespons SELECT 1 di port {}",
            port
        ))
    } else {
        let log = std::fs::read_to_string(db_runtime::mariadb_error_log_path(&home)).unwrap_or_default();
        Err(format!(
            "MariaDB self-test tidak merespons SELECT 1 dalam 30 detik: {}. Log:\n{}",
            last_err,
            tail_of(&log, 1500)
        ))
    }
}

fn host_mariadb_binary_share() -> Result<(PathBuf, PathBuf), String> {
    let binary = crate::runtime::find_in_path("mariadbd")
        .or_else(|| crate::runtime::find_in_path("mysqld"))
        .ok_or_else(|| "mariadbd tidak ditemukan untuk self-test MariaDB".to_string())?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(bin_dir) = binary.parent() {
        if let Some(prefix) = bin_dir.parent() {
            candidates.push(prefix.join("share").join("mysql"));
        }
    }
    candidates.push(PathBuf::from("/usr/share/mysql"));
    for cand in candidates {
        if cand.join("mysql_system_tables.sql").is_file() {
            return Ok((binary, cand));
        }
    }
    Err("Share MariaDB (mysql_system_tables.sql) tidak ditemukan untuk self-test".to_string())
}

async fn run_http_with_lan(
    state: &AppState,
    engine: &str,
    paths: &EnginePaths,
    marker: &str,
) -> EngineOutcome {
    let loopback = if engine == "static" {
        run_static(state, paths, marker, "127.0.0.1", "127.0.0.1").await
    } else {
        run_http_engine(state, engine, paths, marker, "127.0.0.1", "127.0.0.1").await
    };
    let detail = match loopback {
        Ok(detail) => detail,
        Err(error) => return EngineOutcome::err(error),
    };
    let Some(lan_ip) = crate::network::lan_ip() else {
        return EngineOutcome {
            detail,
            error: None,
            lan_ok: None,
            lan_detail: Some(
                "IP LAN perangkat tidak terdeteksi; jalur LAN tidak diuji".to_string(),
            ),
            lan_error: None,
        };
    };
    let lan = if engine == "static" {
        run_static(state, paths, marker, "0.0.0.0", &lan_ip).await
    } else {
        run_http_engine(state, engine, paths, marker, "0.0.0.0", &lan_ip).await
    };
    match lan {
        Ok(lan_detail) => EngineOutcome {
            detail,
            error: None,
            lan_ok: Some(true),
            lan_detail: Some(lan_detail),
            lan_error: None,
        },
        Err(error) => EngineOutcome {
            detail,
            error: Some(format!("Jalur LAN gagal: {}", error)),
            lan_ok: Some(false),
            lan_detail: None,
            lan_error: Some(error),
        },
    }
}

/// Jalankan self-test untuk engine yang diminta (semua bila `engines`
/// kosong/None). Hasil selalu per engine, berurutan sesuai
/// SELFTEST_ENGINES.
pub async fn run_self_test(state: &AppState, engines: Option<Vec<String>>) -> Vec<SelfTestResult> {
    let requested: Vec<&str> = SELFTEST_ENGINES
        .iter()
        .copied()
        .filter(|e| match &engines {
            Some(list) if !list.is_empty() => list.iter().any(|x| x == e),
            _ => true,
        })
        .collect();

    let runtimes = crate::runtime::runtimes_cached(state, false).await;
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S%3f").to_string();
    let base = state.config.data_dir.join("selftest").join(&timestamp);
    let mut results = Vec::new();
    for engine in requested {
        let started = Instant::now();
        let runtime = runtimes.iter().find(|r| r.id == engine);
        let outcome = match runtime {
            Some(r) if !r.available => EngineOutcome::err(
                r.reason
                    .clone()
                    .unwrap_or_else(|| format!("Runtime {} tidak tersedia", engine)),
            ),
            _ => {
                let dir = base.join(engine);
                let marker = selftest_marker(engine);
                let setup = std::fs::create_dir_all(&dir)
                    .and_then(|_| std::fs::write(dir.join("index.html"), selftest_index_html(&marker)));
                match setup {
                    Ok(()) => {
                        let paths = EnginePaths { base: base.clone(), dir: dir.clone() };
                        let outcome = match engine {
                            "postgresql" => match run_postgres_selftest(state, &paths).await {
                                Ok(detail) => EngineOutcome::ok(detail),
                                Err(error) => EngineOutcome::err(error),
                            },
                            "mariadb" => match run_mariadb_selftest(state, &paths).await {
                                Ok(detail) => EngineOutcome::ok(detail),
                                Err(error) => EngineOutcome::err(error),
                            },
                            _ => run_http_with_lan(state, engine, &paths, &marker).await,
                        };
                        let _ = std::fs::remove_dir_all(&dir);
                        outcome
                    }
                    Err(e) => EngineOutcome::err(format!(
                        "Gagal menyiapkan direktori self-test: {}",
                        e
                    )),
                }
            }
        };
        let duration_ms = started.elapsed().as_millis() as u64;
        let mut result = match outcome.error {
            None => ok_result(engine, duration_ms, outcome.detail),
            Some(error) => fail_result(engine, duration_ms, outcome.detail, error),
        };
        result.lan_ok = outcome.lan_ok;
        result.lan_detail = outcome.lan_detail;
        result.lan_error = outcome.lan_error;
        results.push(result);
    }
    let _ = std::fs::remove_dir_all(&base);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_and_index_html() {
        let marker = selftest_marker("php");
        assert!(marker.starts_with("WEBFORGE-SELFTEST-php-"));
        assert!(selftest_index_html(&marker).contains(&marker));
    }

    #[test]
    fn python_script_serves_marker_on_env_port() {
        let script = selftest_python_script("MARKER-123");
        assert!(script.contains("MARKER-123"));
        assert!(script.contains("os.environ.get('PORT'"));
        assert!(script.contains("os.environ.get('HOST', '127.0.0.1')"));
    }

    #[test]
    fn node_script_serves_marker_on_literal_port() {
        let script = selftest_node_script("MARKER-456", 18080, "0.0.0.0");
        assert!(script.contains("MARKER-456"));
        assert!(script.contains("listen(18080, \"0.0.0.0\")"));
    }

    #[test]
    fn launch_commands_per_engine() {
        let docroot = Path::new("/data/selftest/php");
        let php = php_launch(
            Path::new("/lib/libphp.so"),
            "0.0.0.0",
            18081,
            docroot,
            Path::new("/data/selftest/php/php.ini"),
        );
        assert_eq!(
            php.args,
            vec!["-S", "0.0.0.0:18081", "-t", "/data/selftest/php", "-c", "/data/selftest/php/php.ini"]
        );
        assert_eq!(php.cwd, docroot);

        let home = Path::new("/data/selftest/servers/selftest-nginx/nginx");
        let nginx = webserver_launch(
            "nginx",
            Path::new("/lib/libnginx.so"),
            Path::new("/data/selftest/servers/selftest-nginx/nginx/nginx.conf"),
            home,
        );
        assert_eq!(
            nginx.args,
            vec![
                "-c",
                "/data/selftest/servers/selftest-nginx/nginx/nginx.conf",
                "-p",
                "/data/selftest/servers/selftest-nginx/nginx"
            ]
        );
        assert_eq!(nginx.cwd, home);

        let caddy_home = Path::new("/data/selftest/servers/selftest-caddy/caddy");
        let caddy = webserver_launch(
            "caddy",
            Path::new("/lib/libcaddy.so"),
            Path::new("/data/selftest/servers/selftest-caddy/caddy/Caddyfile"),
            caddy_home,
        );
        assert_eq!(caddy.args[0], "run");
        assert!(caddy.envs.iter().any(|(k, _)| k == "XDG_DATA_HOME"));

        let apache = webserver_launch(
            "apache",
            Path::new("/lib/libhttpd.so"),
            Path::new("/data/selftest/servers/selftest-apache/apache/httpd.conf"),
            Path::new("/data/selftest/servers/selftest-apache/apache"),
        );
        assert!(apache.args.iter().any(|a| a == "-DFOREGROUND"));

        let python = python_launch(
            Path::new("/lib/libpython.so"),
            Path::new("/data/selftest/python/server.py"),
            "0.0.0.0",
            18082,
            Vec::new(),
        );
        assert_eq!(python.args, vec!["/data/selftest/python/server.py"]);
        assert!(python.envs.contains(&("HOST".to_string(), "0.0.0.0".to_string())));
        assert!(python.envs.contains(&("PORT".to_string(), "18082".to_string())));
        assert_eq!(python.cwd, PathBuf::from("/data/selftest/python"));

        let node = node_launch(
            Path::new("/usr/bin/node"),
            Path::new("/data/selftest/node/server.js"),
            "0.0.0.0",
            18083,
        );
        assert_eq!(node.args, vec!["/data/selftest/node/server.js"]);
        assert!(node.envs.contains(&("HOST".to_string(), "0.0.0.0".to_string())));
        assert!(node.envs.contains(&("PORT".to_string(), "18083".to_string())));
    }

    #[test]
    fn engines_order_complete() {
        assert_eq!(SELFTEST_ENGINES.len(), 9);
        assert_eq!(SELFTEST_ENGINES[0], "static");
        assert!(SELFTEST_ENGINES.contains(&"postgresql"));
        assert!(SELFTEST_ENGINES.contains(&"mariadb"));
    }
}
