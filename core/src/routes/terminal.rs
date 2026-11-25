use crate::state::AppState;
use crate::util::{db_err, new_id, now_iso, ApiResult};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};

async fn resolve_cwd(state: &AppState, project_id: &Option<String>) -> PathBuf {
    if let Some(pid) = project_id {
        if let Ok(row) = super::projects::get_project_row(&state.pool, pid).await {
            return PathBuf::from(row.get::<String, _>("path"));
        }
    }
    state.config.workspace_dir.clone()
}

async fn save_history(state: &AppState, command: &str, exit_code: i64, stdout: &str, stderr: &str, duration_ms: i64, project_id: &Option<String>) {
    let _ = sqlx::query(
        "INSERT INTO command_history (id, command, exit_code, stdout, stderr, duration_ms, project_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
    )
    .bind(new_id())
    .bind(command)
    .bind(exit_code)
    .bind(stdout.chars().take(20000).collect::<String>())
    .bind(stderr.chars().take(20000).collect::<String>())
    .bind(duration_ms)
    .bind(project_id)
    .bind(now_iso())
    .execute(&state.pool)
    .await;
}

#[derive(Deserialize)]
pub struct ExecuteBody {
    pub command: String,
    pub project_id: Option<String>,
}

pub async fn execute(State(state): State<AppState>, Json(b): Json<ExecuteBody>) -> ApiResult<Json<Value>> {
    let cwd = resolve_cwd(&state, &b.project_id).await;
    let start = Instant::now();
    let fut = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&b.command)
        .current_dir(&cwd)
        .output();
    let (exit_code, stdout, stderr) = match tokio::time::timeout(Duration::from_secs(30), fut).await {
        Ok(Ok(out)) => (
            out.status.code().unwrap_or(-1) as i64,
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        ),
        Ok(Err(e)) => (-1, String::new(), format!("Gagal menjalankan perintah: {}", e)),
        Err(_) => (124, String::new(), "Perintah timeout setelah 30 detik".to_string()),
    };
    let duration_ms = start.elapsed().as_millis() as i64;
    save_history(&state, &b.command, exit_code, &stdout, &stderr, duration_ms, &b.project_id).await;
    Ok(Json(json!({
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "duration_ms": duration_ms,
    })))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

pub async fn history(State(state): State<AppState>, Query(q): Query<HistoryQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let rows = sqlx::query("SELECT id, command, exit_code, created_at FROM command_history ORDER BY created_at DESC LIMIT ?1")
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;
    let mut out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "command": r.get::<String, _>("command"),
                "exit_code": r.get::<Option<i64>, _>("exit_code"),
                "created_at": r.get::<String, _>("created_at"),
            })
        })
        .collect();
    out.reverse();
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
}

pub async fn ws_terminal(
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if q.token.as_deref() != Some(state.config.token.as_str()) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    while let Some(msg) = receiver.next().await {
        let Ok(msg) = msg else { break };
        let Message::Text(text) = msg else { continue };
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if parsed.get("type").and_then(|t| t.as_str()) != Some("run") {
            continue;
        }
        let command = parsed.get("command").and_then(|c| c.as_str()).unwrap_or("").to_string();
        if command.is_empty() {
            continue;
        }
        let project_id = parsed
            .get("project_id")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string());
        let cwd = resolve_cwd(&state, &project_id).await;
        let start = Instant::now();
        let mut child = match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = sender
                    .send(Message::Text(json!({"type":"output","data": format!("Gagal menjalankan perintah: {}\n", e)}).to_string()))
                    .await;
                let _ = sender.send(Message::Text(json!({"type":"exit","code": -1}).to_string())).await;
                continue;
            }
        };
        let mut collected = String::new();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
        if let Some(out) = stdout {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(format!("{}\n", line)).await;
                }
            });
        }
        if let Some(err) = stderr {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(format!("{}\n", line)).await;
                }
            });
        }
        drop(tx);
        let exit_code = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(chunk) = rx.recv().await {
                collected.push_str(&chunk);
                if sender
                    .send(Message::Text(json!({"type":"output","data": chunk}).to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            match child.wait().await {
                Ok(s) => s.code().unwrap_or(-1) as i64,
                Err(_) => -1,
            }
        })
        .await
        .unwrap_or(124);
        let _ = sender.send(Message::Text(json!({"type":"exit","code": exit_code}).to_string())).await;
        save_history(
            &state,
            &command,
            exit_code,
            &collected,
            "",
            start.elapsed().as_millis() as i64,
            &project_id,
        )
        .await;
    }
}
