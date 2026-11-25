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

fn spawn_log_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: R,
    pool: sqlx::SqlitePool,
    server_id: String,
    severity: &'static str,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                app_log(&pool, "server", severity, &line, Some(&server_id)).await;
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
    if let Some(cwd) = &launch.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &launch.envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Gagal menjalankan proses server: {}", e))?;
    let pid = child.id().map(|p| p as i64);
    if let Some(out) = child.stdout.take() {
        spawn_log_reader(out, state.pool.clone(), server.id.clone(), "info");
    }
    if let Some(err) = child.stderr.take() {
        spawn_log_reader(err, state.pool.clone(), server.id.clone(), "error");
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
                        let msg = format!(
                            "Restart otomatis percobaan {} gagal: health check tidak lolos dalam 5 detik",
                            attempts_used
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
                let msg = format!(
                    "Proses server berhenti tak terduga (exit code {}) dan restart otomatis gagal setelah {} percobaan",
                    exit_code.map(|c| c.to_string()).unwrap_or_else(|| "tidak diketahui".to_string()),
                    php::MAX_RESTART_ATTEMPTS
                );
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
            let serve = axum::serve(listener, app);
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

pub async fn start_server(state: &AppState, server: &ServerRow, project_path: &str) -> Result<(), String> {
    let port_u16: u16 = server.port.try_into().map_err(|_| "Port tidak valid".to_string())?;
    let host = if server.bind_lan { "0.0.0.0" } else { "127.0.0.1" };

    // Resolve document root inside project
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

    // Validasi engine sebelum mengalokasikan port.
    let php_binary = match server.engine.as_str() {
        "static" => None,
        "php" => match php::find_php_binary(state.native_lib_dir.as_deref()) {
            Some(b) => Some(b),
            None => {
                let msg = "PHP tidak tersedia: libphp.so tidak ditemukan di nativeLibraryDir dan php tidak ada di PATH".to_string();
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

    // Port conflict: allocation table + real bind test
    if let Err(msg) = allocate_port(&state.pool, &server.id, server.port).await {
        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
        return Err(msg);
    }
    if !port_bind_test(host, port_u16) {
        let msg = format!("Port {} sudah digunakan proses lain", server.port);
        release_port(&state.pool, &server.id).await;
        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
        return Err(msg);
    }

    if let Some(binary) = php_binary {
        // Engine PHP: spawn `php -S host:port -t <docroot> -c <php.ini>`.
        let settings = php::load_php_settings(&state.pool).await;
        let ini_path = match php::write_php_ini(&state.config.data_dir, &settings) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("Gagal menulis php.ini: {}", e);
                release_port(&state.pool, &server.id).await;
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                return Err(msg);
            }
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
        let (pid, child_arc) = match launch_child(state, server, &launch).await {
            Ok(v) => v,
            Err(msg) => {
                release_port(&state.pool, &server.id).await;
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                return Err(msg);
            }
        };
        spawn_child_watcher(state.clone(), server.clone(), launch, child_arc);
        return finish_start(state, server, pid, false).await;
    }

    #[cfg(target_os = "android")]
    {
        // In-process static server task dengan supervisor auto-restart.
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
        return finish_start(state, server, None, true).await;
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
        let (pid, child_arc) = match launch_child(state, server, &launch).await {
            Ok(v) => v,
            Err(msg) => {
                release_port(&state.pool, &server.id).await;
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                return Err(msg);
            }
        };
        spawn_child_watcher(state.clone(), server.clone(), launch, child_arc);
        finish_start(state, server, pid, false).await
    }
}

async fn finish_start(state: &AppState, server: &ServerRow, pid: Option<i64>, in_process: bool) -> Result<(), String> {
    let port_u16 = server.port as u16;
    if wait_healthy(port_u16, Duration::from_secs(5)).await {
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
        let msg = "Health check gagal dalam 5 detik; proses dihentikan".to_string();
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
    fn bind_test_detects_conflict() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!port_bind_test("127.0.0.1", port));
        drop(listener);
        assert!(port_bind_test("127.0.0.1", port));
    }
}
