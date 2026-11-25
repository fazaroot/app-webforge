use crate::state::{app_log, AppState, ProcessHandle};
use crate::util::{new_id, now_iso};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

pub struct ServerRow {
    pub id: String,
    pub project_id: String,
    pub engine: String,
    pub port: i64,
    pub bind_lan: bool,
    pub document_root: String,
    pub status: String,
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

fn spawn_watcher(state: AppState, server_id: String, child_arc: Arc<Mutex<Option<tokio::process::Child>>>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let exited: Option<Option<i32>> = {
                let mut guard = child_arc.lock().await;
                match guard.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            Some(status.code())
                        }
                        Ok(None) => None,
                        Err(_) => Some(None),
                    },
                    None => {
                        // child taken by stop path
                        return;
                    }
                }
            };
            if let Some(code) = exited {
                // Only mark error if DB still thinks it's running/starting (not stopped intentionally)
                let row: Option<(String,)> =
                    sqlx::query_as("SELECT status FROM server_instances WHERE id=?1")
                        .bind(&server_id)
                        .fetch_optional(&state.pool)
                        .await
                        .ok()
                        .flatten();
                if let Some((status,)) = row {
                    if status == "running" || status == "starting" {
                        let msg = format!(
                            "Proses server berhenti tak terduga (exit code {})",
                            code.map(|c| c.to_string()).unwrap_or_else(|| "tidak diketahui".to_string())
                        );
                        set_status(&state.pool, &server_id, "error", None, None, Some(&msg), false).await;
                        app_log(&state.pool, "server", "error", &msg, Some(&server_id)).await;
                        release_port(&state.pool, &server_id).await;
                        let _ = sqlx::query(
                            "UPDATE server_processes SET ended_at=?1, exit_code=?2 WHERE server_id=?3 AND ended_at IS NULL",
                        )
                        .bind(now_iso())
                        .bind(code)
                        .bind(&server_id)
                        .execute(&state.pool)
                        .await;
                    }
                }
                state.processes.lock().await.remove(&server_id);
                return;
            }
        }
    });
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

    #[cfg(target_os = "android")]
    {
        // In-process static server task (cannot exec a child from a .so reliably).
        let listener = tokio::net::TcpListener::bind((host, port_u16))
            .await
            .map_err(|e| e.to_string())?;
        let app = crate::static_app(doc_root.clone());
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let abort = handle.abort_handle();
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
        // health check below with in_process=true
        return finish_start(state, server, None, true).await;
    }

    #[cfg(not(target_os = "android"))]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg("serve-static")
            .arg("--root")
            .arg(&doc_root)
            .arg("--port")
            .arg(server.port.to_string())
            .arg("--host")
            .arg(host)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(false);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("Gagal menjalankan proses server: {}", e);
                release_port(&state.pool, &server.id).await;
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                return Err(msg);
            }
        };
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
        spawn_watcher(state.clone(), server.id.clone(), child_arc);
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
            &format!("Proses static server dimulai (pid {}) di port {}", pid.unwrap_or(-1), server.port),
            Some(&server.id),
        )
        .await;
        finish_start(state, server, pid, false).await
    }
}

async fn finish_start(state: &AppState, server: &ServerRow, pid: Option<i64>, in_process: bool) -> Result<(), String> {
    let port_u16 = server.port as u16;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut healthy = false;
    while tokio::time::Instant::now() < deadline {
        if health_check(port_u16).await {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if healthy {
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
            let mut guard = arc.lock().await;
            if let Some(mut child) = guard.take() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
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
