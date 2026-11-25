use crate::php;
use crate::state::{app_log, AppState, ProcessHandle};
use crate::util::{new_id, now_iso};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct ServerRow {
    pub id: String,
    pub project_id: String,
    pub engine: String,
    pub port: i64,
    pub bind_lan: bool,
    pub document_root: String,
    pub status: String,
    pub config_text: Option<String>,
    pub proxy_port: Option<i64>,
    pub command: Option<String>,
}

/// Spesifikasi proses anak yang bisa diluncurkan ulang oleh watcher
/// saat auto-restart.
#[derive(Clone)]
struct ChildLaunch {
    program: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    envs: Vec<(String, String)>,
    label: String,
}

pub async fn set_status(
    pool: &sqlx::SqlitePool,
    server_id: &str,
    status: &str,
    pid: Option<i64>,
    started_at: Option<&str>,
    error: Option<&str>,
    in_process: bool,
) {
    let _ = sqlx::query(
        "UPDATE server_instances SET status=?1, pid=?2, started_at=?3, error_message=?4, in_process=?5, updated_at=?6 WHERE id=?7",
    )
    .bind(status)
    .bind(pid)
    .bind(started_at)
    .bind(error)
    .bind(if in_process { 1 } else { 0 })
    .bind(now_iso())
    .bind(server_id)
    .execute(pool)
    .await;
}

async fn db_status(pool: &sqlx::SqlitePool, server_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT status FROM server_instances WHERE id=?1")
        .bind(server_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn allocate_port(pool: &sqlx::SqlitePool, server_id: &str, port: i64) -> Result<(), String> {
    sqlx::query("DELETE FROM port_allocations WHERE server_id=?1")
        .bind(server_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO port_allocations (id, port, server_id, allocated_at) VALUES (?1,?2,?3,?4)")
        .bind(new_id())
        .bind(port)
        .bind(server_id)
        .bind(now_iso())
        .execute(pool)
        .await
        .map_err(|_| format!("Port {} sudah dialokasikan server lain", port))?;
    Ok(())
}

async fn release_port(pool: &sqlx::SqlitePool, server_id: &str) {
    let _ = sqlx::query("DELETE FROM port_allocations WHERE server_id=?1")
        .bind(server_id)
        .execute(pool)
        .await;
}

fn port_bind_test(host: &str, port: u16) -> bool {
    std::net::TcpListener::bind((host, port)).is_ok()
}

/// TCP connect + HTTP GET / expecting status < 500.
pub async fn health_check(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = format!("127.0.0.1:{}", port);
    let stream = tokio::time::timeout(Duration::from_millis(800), tokio::net::TcpStream::connect(&addr)).await;
    let mut stream = match stream {
        Ok(Ok(s)) => s,
        _ => return false,
    };
    let req = "GET / HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(req.as_bytes()).await.is_err() {
        return false;
    }
    let mut buf = vec![0u8; 512];
    match tokio::time::timeout(Duration::from_millis(800), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => {
            let text = String::from_utf8_lossy(&buf[..n]);
            if let Some(first) = text.lines().next() {
                // e.g. "HTTP/1.1 200 OK"
                if let Some(code_str) = first.split_whitespace().nth(1) {
                    if let Ok(code) = code_str.parse::<u16>() {
                        return code < 500;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

async fn wait_healthy(port: u16, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if health_check(port).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Kapasitas buffer stderr per proses (baris). error_message hanya
/// mengambil 10 baris terakhir; buffer menyimpan lebih banyak agar
/// konteks kegagalan tidak hilang.
pub(crate) const STDERR_BUFFER_CAP: usize = 50;
const STDERR_LINE_MAX_CHARS: usize = 500;

/// Tambahkan satu baris ke buffer ekor stderr dengan batas kapasitas.
pub(crate) fn push_capped_line(buf: &mut std::collections::VecDeque<String>, line: String) {
    let line: String = line.chars().take(STDERR_LINE_MAX_CHARS).collect();
    buf.push_back(line);
    while buf.len() > STDERR_BUFFER_CAP {
        buf.pop_front();
    }
}

async fn push_stderr_line(state: &AppState, server_id: &str, line: &str) {
    let mut map = state.process_stderr.lock().await;
    let buf = map.entry(server_id.to_string()).or_default();
    push_capped_line(buf, line.to_string());
}

/// Ambil `n` baris stderr terakhir satu server dari buffer in-memory.
async fn stderr_tail(state: &AppState, server_id: &str, n: usize) -> Vec<String> {
    let map = state.process_stderr.lock().await;
    match map.get(server_id) {
        Some(buf) => buf.iter().rev().take(n).rev().cloned().collect(),
        None => Vec::new(),
    }
}

/// Susun error_message health check dari ekor stderr proses (maks 10
/// baris terakhir). Tanpa output stderr, pesan menyatakan itu apa
/// adanya, bukan mengarang penyebab.
pub(crate) fn compose_health_error(timeout_secs: u64, tail: &[String]) -> String {
    let tail: Vec<&String> = tail.iter().rev().take(10).rev().collect();
    if tail.is_empty() {
        format!(
            "Health check gagal dalam {} detik; proses dihentikan. Proses tidak mengeluarkan output stderr",
            timeout_secs
        )
    } else {
        let lines: Vec<&str> = tail.iter().map(|s| s.as_str()).collect();
        format!(
            "Health check gagal dalam {} detik; proses dihentikan. Stderr terakhir:\n{}",
            timeout_secs,
            lines.join("\n")
        )
    }
}

fn spawn_log_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: R,
    state: AppState,
    server_id: String,
    severity: &'static str,
    is_stderr: bool,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                if is_stderr {
                    push_stderr_line(&state, &server_id, &line).await;
                }
                app_log(&state.pool, "server", severity, &line, Some(&server_id)).await;
            }
        }
    });
}

async fn mark_process_ended(pool: &sqlx::SqlitePool, server_id: &str, exit_code: Option<i32>) {
    let _ = sqlx::query(
        "UPDATE server_processes SET ended_at=?1, exit_code=?2 WHERE server_id=?3 AND ended_at IS NULL",
    )
    .bind(now_iso())
    .bind(exit_code)
    .bind(server_id)
    .execute(pool)
    .await;
}

/// Luncurkan proses anak dan daftarkan handle-nya. Mengembalikan
/// (pid, child_arc) agar watcher bisa memantau proses yang sama.
async fn launch_child(
    state: &AppState,
    server: &ServerRow,
    launch: &ChildLaunch,
) -> Result<(Option<i64>, Arc<Mutex<Option<tokio::process::Child>>>), String> {
    let mut cmd = tokio::process::Command::new(&launch.program);
    cmd.args(&launch.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(false);
    // Environment spawn terpusat (LD_LIBRARY_PATH/HOME/TMPDIR + cwd
    // valid); env khusus engine diterapkan sesudahnya dan menang.
    let home = launch
        .envs
        .iter()
        .find(|(k, _)| k == "HOME")
        .map(|(_, v)| PathBuf::from(v))
        .unwrap_or_else(|| state.config.data_dir.clone());
    crate::util::prepare_spawn(
        &mut cmd,
        state.native_lib_dir.as_deref(),
        &home,
        launch.cwd.as_deref(),
    );
    for (k, v) in &launch.envs {
        cmd.env(k, v);
    }
    // Buffer stderr dimulai bersih untuk proses baru ini.
    state.process_stderr.lock().await.remove(&server.id);
    let mut child = cmd.spawn().map_err(|e| format!("Gagal menjalankan proses server: {}", e))?;
    let pid = child.id().map(|p| p as i64);
    if let Some(out) = child.stdout.take() {
        spawn_log_reader(out, state.clone(), server.id.clone(), "info", false);
    }
    if let Some(err) = child.stderr.take() {
        spawn_log_reader(err, state.clone(), server.id.clone(), "error", true);
    }
    let child_arc = Arc::new(Mutex::new(Some(child)));
    state
        .processes
        .lock()
        .await
        .insert(server.id.clone(), ProcessHandle::Child(child_arc.clone()));
    let _ = sqlx::query(
        "INSERT INTO server_processes (id, server_id, pid, started_at) VALUES (?1,?2,?3,?4)",
    )
    .bind(new_id())
    .bind(&server.id)
    .bind(pid)
    .bind(now_iso())
    .execute(&state.pool)
    .await;
    app_log(
        &state.pool,
        "server",
        "info",
        &format!(
            "Proses {} dimulai (pid {}) di port {}",
            launch.label,
            pid.unwrap_or(-1),
            server.port
        ),
        Some(&server.id),
    )
    .await;
    Ok((pid, child_arc))
}

async fn kill_child_arc(child_arc: &Arc<Mutex<Option<tokio::process::Child>>>) {
    let mut guard = child_arc.lock().await;
    if let Some(mut child) = guard.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

/// Pantau proses anak; bila berhenti tak terduga saat status running/
/// starting, restart otomatis dengan backoff 2s/5s/10s (maks 3 kali).
/// Setelah anggaran habis status menjadi error. Stop manual tidak
/// memicu restart karena status berubah ke stopping/stopped.
fn spawn_child_watcher(
    state: AppState,
    server: ServerRow,
    launch: ChildLaunch,
    initial_child: Arc<Mutex<Option<tokio::process::Child>>>,
) {
    tokio::spawn(async move {
        let mut child_arc = initial_child;
        let mut attempts_used: u32 = 0;
        loop {
            // Tunggu proses saat ini berakhir.
            let exit_code: Option<i32> = loop {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let mut guard = child_arc.lock().await;
                match guard.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            break status.code();
                        }
                        Ok(None) => continue,
                        Err(_) => {
                            *guard = None;
                            break None;
                        }
                    },
                    None => {
                        // Child diambil jalur stop — bukan crash.
                        return;
                    }
                }
            };
            mark_process_ended(&state.pool, &server.id, exit_code).await;
            let status = db_status(&state.pool, &server.id).await;
            if !matches!(status.as_deref(), Some("running") | Some("starting")) {
                state.processes.lock().await.remove(&server.id);
                return;
            }
            // Crash tak terduga: coba restart sesuai anggaran.
            let mut restarted = false;
            while let Some(delay) = php::restart_delay(attempts_used) {
                attempts_used += 1;
                let msg = format!(
                    "Proses server berhenti tak terduga (exit code {}); restart otomatis percobaan {}/{} dalam {} detik",
                    exit_code.map(|c| c.to_string()).unwrap_or_else(|| "tidak diketahui".to_string()),
                    attempts_used,
                    php::MAX_RESTART_ATTEMPTS,
                    delay.as_secs()
                );
                app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                set_status(&state.pool, &server.id, "starting", None, None, Some(&msg), false).await;
                tokio::time::sleep(delay).await;
                // Stop manual selama backoff membatalkan restart.
                if db_status(&state.pool, &server.id).await.as_deref() != Some("starting") {
                    return;
                }
                match launch_child(&state, &server, &launch).await {
                    Ok((pid, new_arc)) => {
                        if db_status(&state.pool, &server.id).await.as_deref() != Some("starting") {
                            kill_child_arc(&new_arc).await;
                            state.processes.lock().await.remove(&server.id);
                            return;
                        }
                        if wait_healthy(server.port as u16, Duration::from_secs(5)).await {
                            let started = now_iso();
                            set_status(&state.pool, &server.id, "running", pid, Some(&started), None, false).await;
                            app_log(
                                &state.pool,
                                "server",
                                "info",
                                &format!("Restart otomatis berhasil (percobaan {}), health check lolos", attempts_used),
                                Some(&server.id),
                            )
                            .await;
                            child_arc = new_arc;
                            restarted = true;
                            break;
                        }
                        let tail = stderr_tail(&state, &server.id, 10).await;
                        let msg = format!(
                            "Restart otomatis percobaan {} gagal: {}",
                            attempts_used,
                            compose_health_error(5, &tail)
                        );
                        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                        kill_child_arc(&new_arc).await;
                        mark_process_ended(&state.pool, &server.id, None).await;
                    }
                    Err(e) => {
                        let msg = format!("Restart otomatis percobaan {} gagal: {}", attempts_used, e);
                        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                    }
                }
            }
            if !restarted {
                let tail = stderr_tail(&state, &server.id, 10).await;
                let mut msg = format!(
                    "Proses server berhenti tak terduga (exit code {}) dan restart otomatis gagal setelah {} percobaan",
                    exit_code.map(|c| c.to_string()).unwrap_or_else(|| "tidak diketahui".to_string()),
                    php::MAX_RESTART_ATTEMPTS
                );
                if !tail.is_empty() {
                    msg.push_str(". Stderr terakhir:\n");
                    msg.push_str(&tail.join("\n"));
                }
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                release_port(&state.pool, &server.id).await;
                state.processes.lock().await.remove(&server.id);
                return;
            }
            // Berhasil restart: lanjut pantau proses baru (anggaran tidak reset).
        }
    });
}

/// Supervisor server static in-process (Android). Bila task serve berakhir
/// tak terduga saat status running/starting, bind ulang dengan backoff
/// 2s/5s/10s maks 3 kali; setelah itu status error. Abort dari jalur stop
/// membatalkan supervisor beserta future serve di dalamnya.
#[cfg(target_os = "android")]
fn spawn_inprocess_supervisor(
    state: AppState,
    server: ServerRow,
    host: String,
    doc_root: PathBuf,
) -> tokio::task::AbortHandle {
    let port = server.port as u16;
    let handle = tokio::spawn(async move {
        let mut attempts_used: u32 = 0;
        loop {
            let listener = match tokio::net::TcpListener::bind((host.as_str(), port)).await {
                Ok(l) => l,
                Err(e) => {
                    let status = db_status(&state.pool, &server.id).await;
                    if !matches!(status.as_deref(), Some("running") | Some("starting")) {
                        return;
                    }
                    match php::restart_delay(attempts_used) {
                        Some(delay) => {
                            attempts_used += 1;
                            let msg = format!(
                                "Bind port {} gagal ({}); restart otomatis percobaan {}/{} dalam {} detik",
                                port,
                                e,
                                attempts_used,
                                php::MAX_RESTART_ATTEMPTS,
                                delay.as_secs()
                            );
                            app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                            set_status(&state.pool, &server.id, "starting", None, None, Some(&msg), true).await;
                            tokio::time::sleep(delay).await;
                            if db_status(&state.pool, &server.id).await.as_deref() != Some("starting") {
                                return;
                            }
                            continue;
                        }
                        None => {
                            let msg = format!(
                                "Server in-process gagal bind port {} dan restart otomatis habis setelah {} percobaan: {}",
                                port,
                                php::MAX_RESTART_ATTEMPTS,
                                e
                            );
                            set_status(&state.pool, &server.id, "error", None, None, Some(&msg), true).await;
                            app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                            release_port(&state.pool, &server.id).await;
                            state.processes.lock().await.remove(&server.id);
                            return;
                        }
                    }
                }
            };
            let app = crate::static_app(doc_root.clone());
            // axum::serve mengembalikan Serve (IntoFuture, bukan Future);
            // ubah ke future agar bisa di-pin dan di-select.
            let serve = std::future::IntoFuture::into_future(axum::serve(listener, app));
            tokio::pin!(serve);
            let mut healthy = false;
            let health_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            let serve_result = loop {
                tokio::select! {
                    r = &mut serve => break r.map_err(|e| e.to_string()),
                    _ = tokio::time::sleep(Duration::from_millis(200)), if !healthy => {
                        if health_check(port).await {
                            healthy = true;
                            let status = db_status(&state.pool, &server.id).await;
                            if status.as_deref() == Some("starting") {
                                let started = now_iso();
                                set_status(&state.pool, &server.id, "running", None, Some(&started), None, true).await;
                                app_log(
                                    &state.pool,
                                    "server",
                                    "info",
                                    &format!("Health check lolos, server berjalan di port {}", port),
                                    Some(&server.id),
                                )
                                .await;
                            }
                        } else if tokio::time::Instant::now() > health_deadline {
                            break Err("Health check gagal dalam 5 detik".to_string());
                        }
                    }
                }
            };
            // Task serve berakhir (atau health check gagal).
            let status = db_status(&state.pool, &server.id).await;
            if !matches!(status.as_deref(), Some("running") | Some("starting")) {
                return;
            }
            let reason = match &serve_result {
                Ok(()) => "task server berakhir".to_string(),
                Err(e) => e.clone(),
            };
            match php::restart_delay(attempts_used) {
                Some(delay) => {
                    attempts_used += 1;
                    let msg = format!(
                        "Server in-process berhenti tak terduga ({}); restart otomatis percobaan {}/{} dalam {} detik",
                        reason,
                        attempts_used,
                        php::MAX_RESTART_ATTEMPTS,
                        delay.as_secs()
                    );
                    app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                    set_status(&state.pool, &server.id, "starting", None, None, Some(&msg), true).await;
                    tokio::time::sleep(delay).await;
                    if db_status(&state.pool, &server.id).await.as_deref() != Some("starting") {
                        return;
                    }
                }
                None => {
                    let msg = format!(
                        "Server in-process berhenti tak terduga ({}) dan restart otomatis gagal setelah {} percobaan",
                        reason,
                        php::MAX_RESTART_ATTEMPTS
                    );
                    set_status(&state.pool, &server.id, "error", None, None, Some(&msg), true).await;
                    app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                    release_port(&state.pool, &server.id).await;
                    state.processes.lock().await.remove(&server.id);
                    return;
                }
            }
        }
    });
    handle.abort_handle()
}


fn engine_health_timeout(engine: &str) -> Duration {
    match engine {
        "python" | "node" => Duration::from_secs(25),
        "caddy" | "nginx" | "apache" => Duration::from_secs(10),
        _ => Duration::from_secs(5),
    }
}

pub(crate) fn find_engine_binary(state: &AppState, engine: &str) -> Option<PathBuf> {
    let dir = state.native_lib_dir.as_deref();
    match engine {
        "php" => php::find_php_binary(dir),
        "caddy" => crate::runtime::find_runtime_binary(dir, "libcaddy.so", &["caddy"]).map(|(p, _)| p),
        "nginx" => crate::runtime::find_runtime_binary(dir, "libnginx.so", &["nginx"]).map(|(p, _)| p),
        "apache" => crate::runtime::find_runtime_binary(dir, "libhttpd.so", &["httpd", "apache2"]).map(|(p, _)| p),
        "python" => crate::python_runtime::find_python_binary(dir),
        "node" => crate::runtime::find_in_path("node"),
        _ => None,
    }
}

async fn fail_start(state: &AppState, server: &ServerRow, msg: String) -> String {
    release_port(&state.pool, &server.id).await;
    set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
    app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
    msg
}

async fn launch_and_finish(
    state: &AppState,
    server: &ServerRow,
    launch: ChildLaunch,
) -> Result<(), String> {
    let (pid, child_arc) = match launch_child(state, server, &launch).await {
        Ok(v) => v,
        Err(msg) => return Err(fail_start(state, server, msg).await),
    };
    spawn_child_watcher(state.clone(), server.clone(), launch, child_arc);
    finish_start(state, server, pid, false).await
}

/// Jalankan validasi konfigurasi web server dengan binary sungguhan.
/// Mengembalikan (valid, output gabungan stdout+stderr).
pub async fn validate_webserver_config(
    state: &AppState,
    server: &ServerRow,
    project_path: &str,
    config_override: Option<&str>,
) -> Result<(bool, String), String> {
    let engine = server.engine.as_str();
    if !matches!(engine, "caddy" | "nginx" | "apache") {
        return Err(format!("Engine {} tidak memiliki konfigurasi yang dapat divalidasi", engine));
    }
    let binary = find_engine_binary(state, engine)
        .ok_or_else(|| format!("Runtime {} tidak tersedia", engine))?;
    let doc_root = crate::util::resolve_inside(&PathBuf::from(project_path), &server.document_root, true)
        .map_err(|e| e.message)?;
    let input = crate::webservers::WebServerInput {
        host: if server.bind_lan { "0.0.0.0".to_string() } else { "127.0.0.1".to_string() },
        port: server.port as u16,
        doc_root,
        project_path: PathBuf::from(project_path),
        data_dir: state.config.data_dir.clone(),
        server_id: server.id.clone(),
        proxy_port: server.proxy_port.and_then(|p| u16::try_from(p).ok()),
        custom_config: config_override.map(|s| s.to_string()).or_else(|| server.config_text.clone()),
    };
    let config_path = crate::webservers::write_config(&input, engine)
        .map_err(|e| format!("Gagal menulis konfigurasi: {}", e))?;
    let home = crate::webservers::server_home(&state.config.data_dir, &server.id, engine);
    let args = crate::webservers::validate_args(engine, &config_path, &home);
    let mut validate_cmd = tokio::process::Command::new(&binary);
    validate_cmd.args(&args);
    crate::util::prepare_spawn(
        &mut validate_cmd,
        state.native_lib_dir.as_deref(),
        &home,
        Some(&home),
    );
    let out = tokio::time::timeout(Duration::from_secs(10), validate_cmd.output())
    .await
    .map_err(|_| "Validasi konfigurasi timeout".to_string())?
    .map_err(|e| format!("Gagal menjalankan validasi: {}", e))?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), output.trim().to_string()))
}

pub async fn start_server(state: &AppState, server: &ServerRow, project_path: &str) -> Result<(), String> {
    let port_u16: u16 = server.port.try_into().map_err(|_| "Port tidak valid".to_string())?;
    let host = if server.bind_lan { "0.0.0.0" } else { "127.0.0.1" };

    let doc_root = crate::util::resolve_inside(
        &PathBuf::from(project_path),
        &server.document_root,
        true,
    )
    .map_err(|e| e.message)?;
    if !doc_root.is_dir() {
        let msg = format!("Document root bukan folder: {}", doc_root.display());
        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
        return Err(msg);
    }

    // Validasi engine + ketersediaan binary sebelum mengalokasikan port.
    let engine = server.engine.as_str();
    let binary: Option<PathBuf> = match engine {
        "static" => None,
        "node" => {
            #[cfg(target_os = "android")]
            {
                if crate::runtime::bundled_binary(state.native_lib_dir.as_deref(), "libnode.so").is_none()
                    || crate::runtime::bundled_binary(state.native_lib_dir.as_deref(), "libwebforge_node.so").is_none()
                {
                    let msg = "Node.js tidak tersedia: libnode.so/libwebforge_node.so tidak ditemukan".to_string();
                    set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                    return Err(msg);
                }
                None
            }
            #[cfg(not(target_os = "android"))]
            {
                match find_engine_binary(state, engine) {
                    Some(b) => Some(b),
                    None => {
                        let msg = "Node.js tidak tersedia di PATH".to_string();
                        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                        return Err(msg);
                    }
                }
            }
        }
        "php" | "caddy" | "nginx" | "apache" | "python" => match find_engine_binary(state, engine) {
            Some(b) => Some(b),
            None => {
                let msg = format!("Runtime {} tidak tersedia", engine);
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                return Err(msg);
            }
        },
        other => {
            let msg = format!("Engine {} belum didukung sebagai engine server", other);
            set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
            return Err(msg);
        }
    };

    set_status(&state.pool, &server.id, "starting", None, None, None, false).await;

    if let Err(msg) = allocate_port(&state.pool, &server.id, server.port).await {
        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
        return Err(msg);
    }
    if !port_bind_test(host, port_u16) {
        let msg = format!("Port {} sudah digunakan proses lain", server.port);
        return Err(fail_start(state, server, msg).await);
    }

    match engine {
        "php" => {
            let binary = binary.expect("php binary validated");
            let settings = php::load_php_settings(&state.pool).await;
            let ini_path = match php::write_php_ini(&state.config.data_dir, &settings) {
                Ok(p) => p,
                Err(e) => return Err(fail_start(state, server, format!("Gagal menulis php.ini: {}", e)).await),
            };
            let launch = ChildLaunch {
                program: binary,
                args: php::build_php_server_args(host, port_u16, &doc_root, &ini_path),
                cwd: Some(PathBuf::from(project_path)),
                envs: vec![
                    ("HOME".to_string(), state.config.data_dir.to_string_lossy().to_string()),
                    ("TMPDIR".to_string(), php::php_tmp_dir(&state.config.data_dir).to_string_lossy().to_string()),
                    ("PHPRC".to_string(), state.config.data_dir.to_string_lossy().to_string()),
                ],
                label: "PHP built-in server".to_string(),
            };
            launch_and_finish(state, server, launch).await
        }
        "caddy" | "nginx" | "apache" => {
            let binary = binary.expect("webserver binary validated");
            let input = crate::webservers::WebServerInput {
                host: host.to_string(),
                port: port_u16,
                doc_root: doc_root.clone(),
                project_path: PathBuf::from(project_path),
                data_dir: state.config.data_dir.clone(),
                server_id: server.id.clone(),
                proxy_port: server.proxy_port.and_then(|p| u16::try_from(p).ok()),
                custom_config: server.config_text.clone(),
            };
            let config_path = match crate::webservers::write_config(&input, engine) {
                Ok(p) => p,
                Err(e) => return Err(fail_start(state, server, format!("Gagal menulis konfigurasi: {}", e)).await),
            };
            let home = crate::webservers::server_home(&state.config.data_dir, &server.id, engine);
            let _ = std::fs::create_dir_all(home.join("tmp"));
            // Validasi konfigurasi dengan binary sungguhan sebelum start.
            match validate_webserver_config(state, server, project_path, None).await {
                Ok((true, _)) => {}
                Ok((false, output)) => {
                    let msg = format!("Konfigurasi {} tidak valid: {}", engine, output);
                    return Err(fail_start(state, server, msg).await);
                }
                Err(e) => return Err(fail_start(state, server, e).await),
            }
            let launch = ChildLaunch {
                program: binary,
                args: crate::webservers::launch_args(engine, &config_path, &home),
                cwd: Some(home.clone()),
                envs: vec![
                    ("HOME".to_string(), home.to_string_lossy().to_string()),
                    ("TMPDIR".to_string(), home.join("tmp").to_string_lossy().to_string()),
                    ("XDG_DATA_HOME".to_string(), home.join("data").to_string_lossy().to_string()),
                    ("XDG_CONFIG_HOME".to_string(), home.join("config").to_string_lossy().to_string()),
                ],
                label: format!("{} server", engine),
            };
            launch_and_finish(state, server, launch).await
        }
        "python" => {
            let binary = binary.expect("python binary validated");
            let project = PathBuf::from(project_path);
            let entry = match server.command.as_deref().map(|c| c.trim()).filter(|c| !c.is_empty()) {
                Some(cmd) if cmd.ends_with(".py") => crate::python_runtime::PythonEntry {
                    kind: crate::python_runtime::PythonEntryKind::Script,
                    entry: cmd.to_string(),
                    app_target: None,
                },
                Some("http.server") => crate::python_runtime::PythonEntry {
                    kind: crate::python_runtime::PythonEntryKind::HttpServer,
                    entry: "http.server".to_string(),
                    app_target: None,
                },
                Some(other) => {
                    return Err(fail_start(
                        state,
                        server,
                        format!("Command Python '{}' tidak dikenal; gunakan file .py atau http.server", other),
                    )
                    .await)
                }
                None => crate::python_runtime::detect_python_entry(&project),
            };
            let bundled = crate::python_runtime::is_bundled_python(&binary, state.native_lib_dir.as_deref());
            let mut envs = if bundled {
                crate::python_runtime::python_env(&state.config.data_dir, &server.project_id)
            } else {
                // Python dari PATH (host dev/test): jangan paksa PYTHONHOME
                // ke folder bundle yang tidak ada.
                let mut e = vec![
                    ("HOME".to_string(), state.config.data_dir.to_string_lossy().to_string()),
                    ("TMPDIR".to_string(), state.config.data_dir.join("tmp").to_string_lossy().to_string()),
                ];
                if let Some(sp) = crate::python_runtime::venv_site_packages(&state.config.data_dir, &server.project_id) {
                    e.push(("PYTHONPATH".to_string(), sp.to_string_lossy().to_string()));
                }
                e
            };
            envs.push(("HOST".to_string(), host.to_string()));
            envs.push(("PORT".to_string(), port_u16.to_string()));
            let launch = ChildLaunch {
                program: binary,
                args: crate::python_runtime::build_python_server_args(&entry, host, port_u16, &project),
                cwd: Some(project),
                envs,
                label: "Python server".to_string(),
            };
            launch_and_finish(state, server, launch).await
        }
        "node" => {
            let project = PathBuf::from(project_path);
            let info = crate::node_runtime::inspect_node_project(&project);
            let args = match crate::node_runtime::choose_node_entry(
                &info,
                &project,
                server.command.as_deref(),
                host,
                port_u16,
            ) {
                Ok(a) => a,
                Err(e) => return Err(fail_start(state, server, e).await),
            };
            #[cfg(target_os = "android")]
            {
                start_node_android(state, server, &project, args).await
            }
            #[cfg(not(target_os = "android"))]
            {
                let binary = binary.expect("node binary validated");
                let launch = ChildLaunch {
                    program: binary,
                    args,
                    cwd: Some(project),
                    envs: vec![
                        ("HOST".to_string(), host.to_string()),
                        ("PORT".to_string(), port_u16.to_string()),
                    ],
                    label: "Node.js server".to_string(),
                };
                launch_and_finish(state, server, launch).await
            }
        }
        _ => {
            // static
            #[cfg(target_os = "android")]
            {
                let abort = spawn_inprocess_supervisor(state.clone(), server.clone(), host.to_string(), doc_root.clone());
                state
                    .processes
                    .lock()
                    .await
                    .insert(server.id.clone(), ProcessHandle::InProcess(abort));
                let _ = sqlx::query(
                    "INSERT INTO server_processes (id, server_id, pid, started_at) VALUES (?1,?2,NULL,?3)",
                )
                .bind(new_id())
                .bind(&server.id)
                .bind(now_iso())
                .execute(&state.pool)
                .await;
                finish_start(state, server, None, true).await
            }
            #[cfg(not(target_os = "android"))]
            {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let launch = ChildLaunch {
                    program: exe,
                    args: vec![
                        "serve-static".to_string(),
                        "--root".to_string(),
                        doc_root.to_string_lossy().to_string(),
                        "--port".to_string(),
                        server.port.to_string(),
                        "--host".to_string(),
                        host.to_string(),
                    ],
                    cwd: None,
                    envs: Vec::new(),
                    label: "static server".to_string(),
                };
                launch_and_finish(state, server, launch).await
            }
        }
    }
}

#[cfg(target_os = "android")]
async fn start_node_android(
    state: &AppState,
    server: &ServerRow,
    project: &std::path::Path,
    args: Vec<String>,
) -> Result<(), String> {
    let home = crate::webservers::server_home(&state.config.data_dir, &server.id, "node");
    let _ = std::fs::create_dir_all(&home);
    let log_path = home.join("node.log");
    let status_path = home.join("node-status.txt");
    let _ = std::fs::write(&status_path, "starting");
    let started = crate::jni_bridge::node_start(
        &args,
        &project.to_string_lossy(),
        &log_path.to_string_lossy(),
        &status_path.to_string_lossy(),
    );
    if !started {
        return Err(fail_start(state, server, "NodeService gagal dimulai lewat JNI".to_string()).await);
    }
    let _ = sqlx::query(
        "INSERT INTO server_processes (id, server_id, pid, started_at) VALUES (?1,?2,NULL,?3)",
    )
    .bind(new_id())
    .bind(&server.id)
    .bind(now_iso())
    .execute(&state.pool)
    .await;
    app_log(&state.pool, "server", "info", "NodeService dimulai di proses :node", Some(&server.id)).await;
    let abort = spawn_node_watcher(state.clone(), server.clone(), args, project.to_path_buf(), log_path, status_path);
    state
        .processes
        .lock()
        .await
        .insert(server.id.clone(), ProcessHandle::Node(abort));
    finish_start(state, server, None, false).await
}

/// Watcher Node Android: tail file log ke log center dan auto-restart
/// proses :node bila Node keluar tak terduga (maks 3 kali, backoff sama).
#[cfg(target_os = "android")]
fn spawn_node_watcher(
    state: AppState,
    server: ServerRow,
    args: Vec<String>,
    project: PathBuf,
    log_path: PathBuf,
    status_path: PathBuf,
) -> tokio::task::AbortHandle {
    let handle = tokio::spawn(async move {
        let mut attempts_used: u32 = 0;
        let mut log_offset: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(700)).await;
            // Tail log Node ke application_logs.
            if let Ok(content) = std::fs::read(&log_path) {
                if (content.len() as u64) > log_offset {
                    let new = &content[log_offset as usize..];
                    log_offset = content.len() as u64;
                    for line in String::from_utf8_lossy(new).lines() {
                        if !line.trim().is_empty() {
                            // Log Node menggabungkan stdout+stderr; simpan
                            // juga ke buffer ekor agar kegagalan health
                            // check Node memuat output nyatanya.
                            push_stderr_line(&state, &server.id, line).await;
                            app_log(&state.pool, "server", "info", line, Some(&server.id)).await;
                        }
                    }
                }
            }
            let status = db_status(&state.pool, &server.id).await;
            if !matches!(status.as_deref(), Some("running") | Some("starting")) {
                return;
            }
            let node_status = std::fs::read_to_string(&status_path).unwrap_or_default();
            let node_status = node_status.trim();
            if node_status == "starting" || node_status == "stopped" {
                continue;
            }
            // Status numerik = Node keluar.
            if node_status.parse::<i32>().is_ok() {
                mark_process_ended(&state.pool, &server.id, node_status.parse::<i32>().ok()).await;
                match php::restart_delay(attempts_used) {
                    Some(delay) => {
                        attempts_used += 1;
                        let msg = format!(
                            "Node berhenti tak terduga (exit code {}); restart otomatis percobaan {}/{} dalam {} detik",
                            node_status,
                            attempts_used,
                            php::MAX_RESTART_ATTEMPTS,
                            delay.as_secs()
                        );
                        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                        set_status(&state.pool, &server.id, "starting", None, None, Some(&msg), false).await;
                        tokio::time::sleep(delay).await;
                        if db_status(&state.pool, &server.id).await.as_deref() != Some("starting") {
                            return;
                        }
                        let _ = std::fs::write(&status_path, "starting");
                        let ok = crate::jni_bridge::node_start(
                            &args,
                            &project.to_string_lossy(),
                            &log_path.to_string_lossy(),
                            &status_path.to_string_lossy(),
                        );
                        if ok && wait_healthy(server.port as u16, Duration::from_secs(15)).await {
                            let started = now_iso();
                            set_status(&state.pool, &server.id, "running", None, Some(&started), None, false).await;
                            app_log(&state.pool, "server", "info", "Restart Node berhasil, health check lolos", Some(&server.id)).await;
                        }
                    }
                    None => {
                        let msg = format!(
                            "Node berhenti tak terduga (exit code {}) dan restart otomatis gagal setelah {} percobaan",
                            node_status,
                            php::MAX_RESTART_ATTEMPTS
                        );
                        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                        release_port(&state.pool, &server.id).await;
                        state.processes.lock().await.remove(&server.id);
                        return;
                    }
                }
            }
        }
    });
    handle.abort_handle()
}

async fn finish_start(state: &AppState, server: &ServerRow, pid: Option<i64>, in_process: bool) -> Result<(), String> {
    let port_u16 = server.port as u16;
    let timeout = engine_health_timeout(&server.engine);
    if wait_healthy(port_u16, timeout).await {
        let started = now_iso();
        set_status(&state.pool, &server.id, "running", pid, Some(&started), None, in_process).await;
        app_log(
            &state.pool,
            "server",
            "info",
            &format!("Health check lolos, server berjalan di port {}", server.port),
            Some(&server.id),
        )
        .await;
        Ok(())
    } else {
        // Ambil ekor stderr sebelum proses dihentikan agar error_message
        // memuat penyebab nyata dari proses, bukan pesan generik.
        let tail = stderr_tail(state, &server.id, 10).await;
        let msg = compose_health_error(timeout.as_secs(), &tail);
        stop_process_internal(state, &server.id).await;
        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), in_process).await;
        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
        Err(msg)
    }
}

async fn stop_process_internal(state: &AppState, server_id: &str) {
    let handle = state.processes.lock().await.remove(server_id);
    match handle {
        Some(ProcessHandle::Child(arc)) => {
            kill_child_arc(&arc).await;
        }
        Some(ProcessHandle::InProcess(abort)) => {
            abort.abort();
        }
        Some(ProcessHandle::Node(abort)) => {
            abort.abort();
            #[cfg(target_os = "android")]
            {
                let _ = crate::jni_bridge::node_stop();
            }
        }
        None => {}
    }
    release_port(&state.pool, server_id).await;
    let _ = sqlx::query("UPDATE server_processes SET ended_at=?1 WHERE server_id=?2 AND ended_at IS NULL")
        .bind(now_iso())
        .bind(server_id)
        .execute(&state.pool)
        .await;
}

pub async fn stop_server(state: &AppState, server_id: &str) {
    set_status(&state.pool, server_id, "stopping", None, None, None, false).await;
    stop_process_internal(state, server_id).await;
    set_status(&state.pool, server_id, "stopped", None, None, None, false).await;
    app_log(&state.pool, "server", "info", "Server dihentikan", Some(server_id)).await;
}

/// Reload konfigurasi web server: validasi dulu, lalu restart proses
/// dengan konfigurasi baru. Restart penuh dipakai agar perilaku sama di
/// ketiga engine dan kegagalan validasi tidak menjatuhkan server lama.
pub async fn reload_server(state: &AppState, server: &ServerRow, project_path: &str) -> Result<(), String> {
    if !matches!(server.engine.as_str(), "caddy" | "nginx" | "apache") {
        return Err(format!("Engine {} tidak mendukung reload konfigurasi", server.engine));
    }
    let (valid, output) = validate_webserver_config(state, server, project_path, None).await?;
    if !valid {
        return Err(format!("Konfigurasi tidak valid, reload dibatalkan: {}", output));
    }
    stop_server(state, &server.id).await;
    start_server(state, server, project_path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_check_false_on_closed_port() {
        // Find a free port then close it
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!health_check(port).await);
    }

    #[test]
    fn health_error_includes_last_ten_stderr_lines() {
        let tail: Vec<String> = (1..=15).map(|i| format!("baris-{}", i)).collect();
        let msg = compose_health_error(5, &tail);
        assert!(msg.contains("Health check gagal dalam 5 detik"));
        assert!(msg.contains("Stderr terakhir:"));
        assert!(!msg.contains("baris-5\n"), "hanya 10 baris terakhir: {}", msg);
        assert!(msg.contains("baris-6"));
        assert!(msg.contains("baris-15"));
    }

    #[test]
    fn health_error_without_stderr_says_so() {
        let msg = compose_health_error(10, &[]);
        assert!(msg.contains("tidak mengeluarkan output stderr"));
    }

    #[test]
    fn stderr_buffer_caps_at_capacity() {
        let mut buf = std::collections::VecDeque::new();
        for i in 0..(STDERR_BUFFER_CAP + 5) {
            push_capped_line(&mut buf, format!("baris-{}", i));
        }
        assert_eq!(buf.len(), STDERR_BUFFER_CAP);
        assert_eq!(buf.front().map(|s| s.as_str()), Some("baris-5"));
    }

    #[test]
    fn bind_test_detects_conflict() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!port_bind_test("127.0.0.1", port));
        drop(listener);
        assert!(port_bind_test("127.0.0.1", port));
    }
}
