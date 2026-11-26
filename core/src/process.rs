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
    pub backend_port: Option<i64>,
    pub health_failures: i64,
    pub auto_restart_count: i64,
    /// Pilihan versi PHP server ini (None = warisi proyek).
    pub php_version: Option<String>,
    /// Pilihan versi PHP proyek induk (None = aturan default).
    pub project_php_version: Option<String>,
}

impl ServerRow {
    /// Versi PHP efektif: pilihan server menang, lalu proyek, lalu
    /// aturan default di php::resolve_php. Rekomendasi framework tidak
    /// pernah menimpa pilihan ini diam-diam.
    pub fn effective_php_version(&self) -> Option<&str> {
        self.php_version
            .as_deref()
            .or(self.project_php_version.as_deref())
    }
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

/// Target probe HTTP yang benar-benar dipakai health check/supervisi.
/// Untuk server LAN targetnya IP LAN perangkat + Host LAN; untuk server
/// lokal targetnya loopback. `fallback_note` diisi bila bind_lan aktif
/// tetapi IP LAN tidak terdeteksi sehingga probe terpaksa loopback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    pub host: String,
    pub port: u16,
    pub host_header: String,
    pub label: String,
    pub fallback_note: Option<String>,
}

pub fn probe_target_for(bind_lan: bool, port: u16) -> ProbeTarget {
    probe_target_with_lan(bind_lan, port, crate::network::lan_ip())
}

pub(crate) fn probe_target_with_lan(bind_lan: bool, port: u16, lan_ip: Option<String>) -> ProbeTarget {
    if bind_lan {
        if let Some(ip) = lan_ip.filter(|ip| !ip.trim().is_empty()) {
            return ProbeTarget {
                host_header: format!("{}:{}", ip, port),
                label: format!("http://{}:{}", ip, port),
                host: ip,
                port,
                fallback_note: None,
            };
        }
        return ProbeTarget {
            host: "127.0.0.1".to_string(),
            port,
            host_header: format!("127.0.0.1:{}", port),
            label: format!("http://127.0.0.1:{}", port),
            fallback_note: Some(
                "IP LAN tidak terdeteksi; probe memakai loopback dan jalur LAN belum terverifikasi"
                    .to_string(),
            ),
        };
    }
    ProbeTarget {
        host: "127.0.0.1".to_string(),
        port,
        host_header: format!("127.0.0.1:{}", port),
        label: format!("http://127.0.0.1:{}", port),
        fallback_note: None,
    }
}

fn loopback_target(port: u16) -> ProbeTarget {
    probe_target_with_lan(false, port, None)
}

/// Klasifikasi hasil probe untuk supervisi kesehatan:
/// - `Alive`: respons HTTP apa pun <500. Server terbukti hidup;
///   403/404 adalah peringatan docroot, bukan kegagalan, dan tidak
///   pernah memicu restart.
/// - `ServerError`: respons HTTP 5xx. Kegagalan yang boleh memicu
///   degraded/restart bila beruntun.
/// - `TransportFailure`: timeout/refused/reset/tanpa respons HTTP.
///   Kegagalan yang boleh memicu degraded/restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeClass {
    Alive,
    ServerError,
    TransportFailure,
}

impl ProbeClass {
    pub fn is_alive(self) -> bool {
        self == ProbeClass::Alive
    }
    pub fn is_failure(self) -> bool {
        !self.is_alive()
    }
}

/// Hasil satu probe HTTP nyata. `ok` berarti klasifikasi `Alive`
/// (server menjawab HTTP dengan status <500); status 403/404 tetap
/// `ok` sebagai bukti proses menjawab, tetapi pemanggil wajib
/// menurunkannya menjadi peringatan docroot bila akar situs tidak
/// bisa dilayani (lihat `probe_warning`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpProbe {
    pub ok: bool,
    pub reachable: bool,
    pub status_code: Option<u16>,
    pub duration_ms: u64,
    pub target: String,
    pub error: Option<String>,
}

impl HttpProbe {
    pub fn class(&self) -> ProbeClass {
        match self.status_code {
            Some(code) if code < 500 => ProbeClass::Alive,
            Some(_) => ProbeClass::ServerError,
            None => ProbeClass::TransportFailure,
        }
    }
}

impl HttpProbe {
    pub fn evidence(&self) -> String {
        match (self.status_code, &self.error) {
            (Some(code), _) => format!(
                "Probe {} menjawab HTTP {} dalam {} ms",
                self.target, code, self.duration_ms
            ),
            (None, Some(error)) => format!(
                "Probe {} gagal dalam {} ms: {}",
                self.target, self.duration_ms, error
            ),
            (None, None) => format!("Probe {} gagal dalam {} ms", self.target, self.duration_ms),
        }
    }
}

pub(crate) fn parse_http_status(text: &str) -> Option<u16> {
    let first = text.lines().next()?;
    let mut parts = first.split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse::<u16>().ok()
}

pub async fn probe_http_target(target: &ProbeTarget, timeout: Duration) -> HttpProbe {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let started = tokio::time::Instant::now();
    let addr = format!("{}:{}", target.host, target.port);
    let connect_timeout = timeout.min(Duration::from_millis(1500));
    let stream = tokio::time::timeout(
        connect_timeout,
        tokio::net::TcpStream::connect(&addr),
    )
    .await;
    let mut stream = match stream {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return HttpProbe {
                ok: false,
                reachable: false,
                status_code: None,
                duration_ms: started.elapsed().as_millis() as u64,
                target: target.label.clone(),
                error: Some(format!("koneksi TCP ke {} gagal: {}", addr, e)),
            }
        }
        Err(_) => {
            return HttpProbe {
                ok: false,
                reachable: false,
                status_code: None,
                duration_ms: started.elapsed().as_millis() as u64,
                target: target.label.clone(),
                error: Some(format!("timeout koneksi TCP ke {}", addr)),
            }
        }
    };
    let req = format!(
        "GET / HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
        target.host_header
    );
    if let Err(e) = stream.write_all(req.as_bytes()).await {
        return HttpProbe {
            ok: false,
            reachable: false,
            status_code: None,
            duration_ms: started.elapsed().as_millis() as u64,
            target: target.label.clone(),
            error: Some(format!("gagal mengirim request HTTP ke {}: {}", addr, e)),
        };
    }
    let mut buf = vec![0u8; 512];
    let read_timeout = timeout.min(Duration::from_millis(1500));
    match tokio::time::timeout(read_timeout, stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => {
            let text = String::from_utf8_lossy(&buf[..n]);
            match parse_http_status(&text) {
                Some(code) => HttpProbe {
                    ok: code < 500,
                    reachable: true,
                    status_code: Some(code),
                    duration_ms: started.elapsed().as_millis() as u64,
                    target: target.label.clone(),
                    error: if code < 500 {
                        None
                    } else {
                        Some(format!("server menjawab HTTP {}", code))
                    },
                },
                None => HttpProbe {
                    ok: false,
                    reachable: false,
                    status_code: None,
                    duration_ms: started.elapsed().as_millis() as u64,
                    target: target.label.clone(),
                    error: Some("respons bukan HTTP yang valid".to_string()),
                },
            }
        }
        Ok(Ok(_)) => HttpProbe {
            ok: false,
            reachable: false,
            status_code: None,
            duration_ms: started.elapsed().as_millis() as u64,
            target: target.label.clone(),
            error: Some("koneksi ditutup tanpa respons HTTP".to_string()),
        },
        Ok(Err(e)) => HttpProbe {
            ok: false,
            reachable: false,
            status_code: None,
            duration_ms: started.elapsed().as_millis() as u64,
            target: target.label.clone(),
            error: Some(format!("gagal membaca respons HTTP dari {}: {}", addr, e)),
        },
        Err(_) => HttpProbe {
            ok: false,
            reachable: false,
            status_code: None,
            duration_ms: started.elapsed().as_millis() as u64,
            target: target.label.clone(),
            error: Some(format!("timeout membaca respons HTTP dari {}", addr)),
        },
    }
}

/// TCP connect + HTTP GET / expecting status < 500 (kompatibilitas lama).
pub async fn health_check(port: u16) -> bool {
    probe_http_target(&loopback_target(port), Duration::from_millis(1600))
        .await
        .ok
}

async fn wait_probe_healthy(target: &ProbeTarget, timeout: Duration) -> HttpProbe {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = probe_http_target(target, Duration::from_millis(1600)).await;
    // Alive (termasuk 403/404) menghentikan penantian: server terbukti
    // menjawab. Hanya 5xx dan kegagalan transport yang ditunggu.
    while last.class().is_failure() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        last = probe_http_target(target, Duration::from_millis(1600)).await;
    }
    last
}

async fn wait_server_healthy(server: &ServerRow, timeout: Duration) -> HttpProbe {
    let target = probe_target_for(server.bind_lan, server.port as u16);
    wait_probe_healthy(&target, timeout).await
}

/// True bila proyek terdeteksi PHP (framework PHP atau index.php di
/// proyek/docroot). Dipakai mode komposit dan peringatan Apache+PHP.
pub(crate) fn is_php_project(project_path: &std::path::Path, doc_root: &std::path::Path) -> bool {
    let mut available = std::collections::HashSet::new();
    available.insert("php".to_string());
    let info = crate::framework::detect(project_path, &available);
    info.language == "PHP"
        || doc_root.join("index.php").is_file()
        || project_path.join("index.php").is_file()
        || project_path.join("public").join("index.php").is_file()
}

/// Peringatan docroot jujur. `None` berarti akar situs mempunyai index
/// yang bisa dilayani engine ini (atau backend proxy yang menangani).
pub(crate) fn docroot_warning(
    engine: &str,
    project_path: &std::path::Path,
    doc_root: &std::path::Path,
    effective_proxy_port: Option<u16>,
) -> Option<String> {
    if effective_proxy_port.is_some() {
        return None;
    }
    match engine {
        "python" | "node" => None,
        "php" => {
            if doc_root.join("index.php").is_file() {
                None
            } else {
                Some(
                    "Docroot tidak memiliki index.php; request / dapat menghasilkan 404. Periksa document root proyek PHP"
                        .to_string(),
                )
            }
        }
        "static" | "caddy" | "nginx" | "apache" => {
            if doc_root.join("index.html").is_file() || doc_root.join("index.htm").is_file() {
                return None;
            }
            if doc_root.join("index.php").is_file() || is_php_project(project_path, doc_root) {
                if engine == "static" {
                    Some(
                        "Server static tidak mengeksekusi PHP; index.php tidak akan berjalan. Gunakan engine PHP atau server komposit Nginx/Caddy/Apache dengan PHP-FPM"
                            .to_string(),
                    )
                } else {
                    Some(
                        "Docroot tidak memiliki index.html; proyek PHP butuh engine PHP atau mode komposit PHP-FPM (Nginx/Caddy/Apache) agar index.php dieksekusi"
                            .to_string(),
                    )
                }
            } else {
                Some(
                    "Docroot tidak memiliki index.html/index.htm; request / dapat menghasilkan 403/404"
                        .to_string(),
                )
            }
        }
        _ => None,
    }
}

/// Turunkan peringatan dari hasil probe + keadaan docroot. HTTP 403 pada
/// `/` bukan sehat penuh: server tetap running hanya dengan peringatan.
pub(crate) fn probe_warning(probe: &HttpProbe, docroot_warning: Option<String>) -> Option<String> {
    match probe.status_code {
        Some(403) => Some(docroot_warning.unwrap_or_else(|| {
            "Server menjawab HTTP 403 pada /; periksa index document root dan izin folder proyek"
                .to_string()
        })),
        Some(404) => docroot_warning.or_else(|| {
            Some("Server menjawab HTTP 404 pada /; periksa document root dan index proyek".to_string())
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HealthAction {
    None,
    MarkDegraded,
    Restart,
    MarkError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HealthTransition {
    pub status: &'static str,
    pub failures: i64,
    pub action: HealthAction,
}

/// Mesin transisi supervisi kesehatan (murni, untuk test): satu kegagalan
/// belum mengubah status, dua kegagalan beruntun menjadi degraded, tiga
/// memicu restart (maks 3), dan setelah anggaran restart habis menjadi
/// error. Probe `Alive` (termasuk 403/404) selalu memulihkan ke running
/// dan mereset hitungan kegagalan — respons HTTP apa pun <500 adalah
/// bukti server hidup, bukan alasan restart.
pub(crate) fn next_health_transition(
    current_status: &str,
    failures: i64,
    probe_class: ProbeClass,
    auto_restart_count: i64,
) -> HealthTransition {
    if probe_class.is_alive() {
        return HealthTransition {
            status: "running",
            failures: 0,
            action: HealthAction::None,
        };
    }
    let failures = failures.saturating_add(1);
    if failures < 2 {
        return HealthTransition {
            status: if current_status == "degraded" { "degraded" } else { "running" },
            failures,
            action: HealthAction::None,
        };
    }
    if failures < 3 {
        return HealthTransition {
            status: "degraded",
            failures,
            action: if current_status == "degraded" {
                HealthAction::None
            } else {
                HealthAction::MarkDegraded
            },
        };
    }
    if auto_restart_count < php::MAX_RESTART_ATTEMPTS as i64 {
        HealthTransition {
            status: "degraded",
            failures,
            action: HealthAction::Restart,
        }
    } else {
        HealthTransition {
            status: "error",
            failures,
            action: HealthAction::MarkError,
        }
    }
}

async fn update_health_fields(
    pool: &sqlx::SqlitePool,
    server_id: &str,
    probe: &HttpProbe,
    failures: i64,
    warning: Option<&str>,
) {
    let _ = sqlx::query(
        "UPDATE server_instances SET last_health_at=?1, last_health_status=?2, last_health_error=?3, last_health_target=?4, health_failures=?5, warning_message=?6, updated_at=?1 WHERE id=?7",
    )
    .bind(now_iso())
    .bind(probe.status_code.map(|c| c as i64))
    .bind(probe.error.as_deref())
    .bind(&probe.target)
    .bind(failures)
    .bind(warning)
    .bind(server_id)
    .execute(pool)
    .await;
}

async fn set_running_with_probe(
    pool: &sqlx::SqlitePool,
    server_id: &str,
    pid: Option<i64>,
    in_process: bool,
    probe: &HttpProbe,
    warning: Option<&str>,
    reset_restart_budget: bool,
) {
    let started = now_iso();
    let _ = sqlx::query(
        "UPDATE server_instances SET status='running', pid=?1, started_at=?2, error_message=NULL, warning_message=?3, in_process=?4, last_health_at=?2, last_health_status=?5, last_health_error=?6, last_health_target=?7, health_failures=0, auto_restart_count=CASE WHEN ?8 THEN 0 ELSE auto_restart_count END, updated_at=?2 WHERE id=?9",
    )
    .bind(pid)
    .bind(&started)
    .bind(warning)
    .bind(if in_process { 1 } else { 0 })
    .bind(probe.status_code.map(|c| c as i64))
    .bind(probe.error.as_deref())
    .bind(&probe.target)
    .bind(if reset_restart_budget { 1 } else { 0 })
    .bind(server_id)
    .execute(pool)
    .await;
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

/// Spawn proses anak tanpa mendaftarkan handle (mode komposit
/// mendaftarkan frontend+backend sebagai satu handle sesudahnya).
async fn spawn_child_arc(
    state: &AppState,
    server: &ServerRow,
    launch: &ChildLaunch,
    clear_stderr: bool,
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
    // Buffer stderr dimulai bersih untuk proses baru ini (backend
    // komposit tidak membersihkan buffer frontend yang baru dimulai).
    if clear_stderr {
        state.process_stderr.lock().await.remove(&server.id);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Gagal menjalankan proses server: {}", e))?;
    let pid = child.id().map(|p| p as i64);
    if let Some(out) = child.stdout.take() {
        spawn_log_reader(out, state.clone(), server.id.clone(), "info", false);
    }
    if let Some(err) = child.stderr.take() {
        spawn_log_reader(err, state.clone(), server.id.clone(), "error", true);
    }
    let child_arc = Arc::new(Mutex::new(Some(child)));
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

/// Luncurkan proses anak dan daftarkan handle-nya. Mengembalikan
/// (pid, child_arc) agar watcher bisa memantau proses yang sama.
async fn launch_child(
    state: &AppState,
    server: &ServerRow,
    launch: &ChildLaunch,
) -> Result<(Option<i64>, Arc<Mutex<Option<tokio::process::Child>>>), String> {
    let (pid, child_arc) = spawn_child_arc(state, server, launch, true).await?;
    state
        .processes
        .lock()
        .await
        .insert(server.id.clone(), ProcessHandle::Child(child_arc.clone()));
    Ok((pid, child_arc))
}

async fn kill_child_arc(child_arc: &Arc<Mutex<Option<tokio::process::Child>>>) {
    let mut guard = child_arc.lock().await;
    if let Some(mut child) = guard.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

/// Pecah chunk bytes tail log menjadi baris lengkap; sisa baris parsial
/// tetap di `buffer` untuk chunk berikutnya. Buffer dibatasi agar baris
/// tanpa newline tidak tumbuh tanpa batas.
pub(crate) fn parse_log_tail_chunk(buffer: &mut String, bytes: &[u8]) -> Vec<String> {
    buffer.push_str(&String::from_utf8_lossy(bytes));
    let mut lines = Vec::new();
    while let Some(pos) = buffer.find('\n') {
        let line: String = buffer.drain(..=pos).collect();
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        if !line.trim().is_empty() {
            lines.push(line.chars().take(1000).collect());
        }
    }
    if buffer.len() > 8192 {
        let line: String = buffer.chars().take(1000).collect();
        lines.push(line);
        buffer.clear();
    }
    lines
}

pub(crate) fn nginx_log_severity(line: &str) -> &'static str {
    if line.contains("[emerg]") || line.contains("[alert]") || line.contains("[crit]") || line.contains("[error]") {
        "error"
    } else if line.contains("[warn]") {
        "warn"
    } else {
        "info"
    }
}

fn apache_log_severity(line: &str) -> &'static str {
    let lower = line.to_ascii_lowercase();
    if lower.contains(":error]") || lower.contains("error") || lower.contains("crit") || lower.contains("alert") {
        "error"
    } else if lower.contains(":warn]") || lower.contains("warn") {
        "warn"
    } else {
        "info"
    }
}

/// File log yang di-tail ke log center per engine: (path, label, jenis).
fn log_tail_files(engine: &str, home: &std::path::Path) -> Vec<(PathBuf, &'static str, &'static str)> {
    match engine {
        "apache" => vec![
            (home.join("logs").join("error.log"), "apache error", "apache-error"),
            (home.join("logs").join("access.log"), "apache access", "access"),
        ],
        "nginx" => vec![
            (home.join("logs").join("error.log"), "nginx error", "nginx-error"),
            (home.join("logs").join("access.log"), "nginx access", "access"),
        ],
        _ => Vec::new(),
    }
}

async fn stop_log_tails(state: &AppState, server_id: &str) {
    if let Some(tasks) = state.log_tail_tasks.lock().await.remove(server_id) {
        for task in tasks {
            task.abort();
        }
    }
}

/// Mulai tail file log Apache/Nginx ke application_logs. Offset disimpan
/// per file di AppState sehingga restart server tidak menduplikasi baris
/// lama; file yang terpotong (len < offset) dibaca ulang dari awal.
fn start_log_tails(state: &AppState, server: &ServerRow) {
    if !matches!(server.engine.as_str(), "apache" | "nginx") {
        return;
    }
    let home = crate::webservers::server_home(&state.config.data_dir, &server.id, &server.engine);
    let files = log_tail_files(&server.engine, &home);
    let state = state.clone();
    let server_id = server.id.clone();
    tokio::spawn(async move {
        stop_log_tails(&state, &server_id).await;
        let mut handles = Vec::new();
        for (path, label, kind) in files {
            let state = state.clone();
            let server_id = server_id.clone();
            let handle = tokio::spawn(async move {
                let key = format!("{}:{}", server_id, path.to_string_lossy());
                let mut buffer = String::new();
                loop {
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                    let metadata = match std::fs::metadata(&path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let len = metadata.len();
                    let mut offset = {
                        let offsets = state.log_tail_offsets.lock().await;
                        offsets.get(&key).copied().unwrap_or(0)
                    };
                    if len < offset {
                        offset = 0;
                        buffer.clear();
                    }
                    if len == offset {
                        continue;
                    }
                    use std::io::{Read, Seek, SeekFrom};
                    let mut file = match std::fs::File::open(&path) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    if file.seek(SeekFrom::Start(offset)).is_err() {
                        continue;
                    }
                    let mut bytes = Vec::new();
                    if file.read_to_end(&mut bytes).is_err() {
                        continue;
                    }
                    offset += bytes.len() as u64;
                    state.log_tail_offsets.lock().await.insert(key.clone(), offset);
                    for line in parse_log_tail_chunk(&mut buffer, &bytes) {
                        let severity = match kind {
                            "nginx-error" => nginx_log_severity(&line),
                            "apache-error" => apache_log_severity(&line),
                            _ => "info",
                        };
                        if severity == "error" {
                            push_stderr_line(&state, &server_id, &line).await;
                        }
                        app_log(
                            &state.pool,
                            "server",
                            severity,
                            &format!("[{}] {}", label, line),
                            Some(&server_id),
                        )
                        .await;
                    }
                }
            });
            handles.push(handle.abort_handle());
        }
        state.log_tail_tasks.lock().await.insert(server_id, handles);
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessLiveness {
    Alive,
    /// Proses keluar tetapi watcher khusus engine sedang menanganinya.
    ExitedHandled,
    Missing,
}

async fn child_liveness(child_arc: &Arc<Mutex<Option<tokio::process::Child>>>) -> ProcessLiveness {
    let mut guard = child_arc.lock().await;
    match guard.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(None) => ProcessLiveness::Alive,
            Ok(Some(_)) => ProcessLiveness::ExitedHandled,
            Err(_) => ProcessLiveness::Missing,
        },
        None => ProcessLiveness::Missing,
    }
}

async fn process_liveness(state: &AppState, server: &ServerRow) -> ProcessLiveness {
    let handle = {
        let processes = state.processes.lock().await;
        match processes.get(&server.id) {
            Some(ProcessHandle::Child(arc)) => Some(arc.clone()),
            _ => None,
        }
    };
    if let Some(arc) = handle {
        return child_liveness(&arc).await;
    }
    let processes = state.processes.lock().await;
    match processes.get(&server.id) {
        Some(ProcessHandle::Composite { frontend, backend }) => {
            let frontend = frontend.clone();
            let backend = backend.clone();
            drop(processes);
            let f = child_liveness(&frontend).await;
            let b = child_liveness(&backend).await;
            if f == ProcessLiveness::Alive && b == ProcessLiveness::Alive {
                ProcessLiveness::Alive
            } else {
                // Komposit tidak punya watcher exit khusus; supervisor
                // yang me-restart keseluruhan unit.
                ProcessLiveness::Missing
            }
        }
        Some(ProcessHandle::InProcess(_)) => ProcessLiveness::Alive,
        Some(ProcessHandle::Node(_)) => {
            let home = crate::webservers::server_home(&state.config.data_dir, &server.id, "node");
            let status = std::fs::read_to_string(home.join("node-status.txt")).unwrap_or_default();
            if status.trim().parse::<i32>().is_ok() {
                ProcessLiveness::ExitedHandled
            } else {
                ProcessLiveness::Alive
            }
        }
        Some(ProcessHandle::Child(_)) => ProcessLiveness::Missing,
        None => ProcessLiveness::Missing,
    }
}

async fn server_row_for_supervisor(
    pool: &sqlx::SqlitePool,
    server_id: &str,
) -> Option<(ServerRow, String)> {
    let row = sqlx::query(
        "SELECT s.*, p.path AS project_path, p.php_version AS project_php_version FROM server_instances s JOIN projects p ON p.id=s.project_id WHERE s.id=?1",
    )
    .bind(server_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    use sqlx::Row;
    Some((
        ServerRow {
            id: row.get("id"),
            project_id: row.get("project_id"),
            engine: row.get("engine"),
            port: row.get("port"),
            bind_lan: row.get::<i64, _>("bind_lan") != 0,
            document_root: row.get("document_root"),
            status: row.get("status"),
            config_text: row.try_get("config_text").ok().flatten(),
            proxy_port: row.try_get("proxy_port").ok().flatten(),
            command: row.try_get("command").ok().flatten(),
            backend_port: row.try_get("backend_port").ok().flatten(),
            health_failures: row.try_get("health_failures").unwrap_or(0),
            auto_restart_count: row.try_get("auto_restart_count").unwrap_or(0),
            php_version: row.try_get("php_version").ok().flatten(),
            project_php_version: row.try_get("project_php_version").ok().flatten(),
        },
        row.get("project_path"),
    ))
}

fn supervisor_restart_delay(attempts_used: i64) -> Duration {
    match attempts_used {
        0 => Duration::from_secs(2),
        1 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

fn spawn_supervisor_restart(state: AppState, server_id: String, evidence: String) {
    tokio::spawn(async move {
        {
            let mut guard = state.health_restarting.lock().await;
            if !guard.insert(server_id.clone()) {
                return;
            }
        }
        let result = supervisor_restart_loop(&state, &server_id, &evidence).await;
        if let Err(msg) = result {
            app_log(&state.pool, "server", "error", &msg, Some(&server_id)).await;
        }
        state.health_restarting.lock().await.remove(&server_id);
    });
}

async fn supervisor_restart_loop(
    state: &AppState,
    server_id: &str,
    evidence: &str,
) -> Result<(), String> {
    loop {
        let (server, project_path) = match server_row_for_supervisor(&state.pool, server_id).await {
            Some(v) => v,
            None => return Ok(()),
        };
        if !matches!(server.status.as_str(), "running" | "degraded" | "starting") {
            return Ok(());
        }
        if server.auto_restart_count >= php::MAX_RESTART_ATTEMPTS as i64 {
            let msg = format!(
                "Supervisi kesehatan: server tidak menjawab HTTP dan restart otomatis gagal setelah {} percobaan. Bukti probe terakhir: {}",
                php::MAX_RESTART_ATTEMPTS,
                evidence
            );
            stop_process_internal(state, server_id).await;
            set_status(&state.pool, server_id, "error", None, None, Some(&msg), false).await;
            app_log(&state.pool, "server", "error", &msg, Some(server_id)).await;
            return Ok(());
        }
        let attempt = server.auto_restart_count + 1;
        let delay = supervisor_restart_delay(server.auto_restart_count);
        let msg = format!(
            "Supervisi kesehatan: restart otomatis percobaan {}/{} dalam {} detik. Bukti probe terakhir: {}",
            attempt,
            php::MAX_RESTART_ATTEMPTS,
            delay.as_secs(),
            evidence
        );
        let _ = sqlx::query(
            "UPDATE server_instances SET status='starting', error_message=?1, auto_restart_count=?2, updated_at=?3 WHERE id=?4",
        )
        .bind(&msg)
        .bind(attempt)
        .bind(now_iso())
        .bind(server_id)
        .execute(&state.pool)
        .await;
        app_log(&state.pool, "server", "error", &msg, Some(server_id)).await;
        tokio::time::sleep(delay).await;
        if db_status(&state.pool, server_id).await.as_deref() != Some("starting") {
            return Ok(());
        }
        stop_process_internal(state, server_id).await;
        let (fresh, _) = match server_row_for_supervisor(&state.pool, server_id).await {
            Some(v) => v,
            None => return Ok(()),
        };
        match start_server_inner(state, &fresh, &project_path, false).await {
            Ok(()) => {
                app_log(
                    &state.pool,
                    "server",
                    "info",
                    &format!(
                        "Supervisi kesehatan: restart otomatis percobaan {} berhasil; server menjawab HTTP lagi",
                        attempt
                    ),
                    Some(server_id),
                )
                .await;
                return Ok(());
            }
            Err(e) => {
                let msg = format!(
                    "Supervisi kesehatan: restart otomatis percobaan {} gagal: {}",
                    attempt, e
                );
                app_log(&state.pool, "server", "error", &msg, Some(server_id)).await;
                // start_server menandai error; kembalikan ke degraded agar
                // loop anggaran restart berikutnya tetap berjalan sampai
                // maks 3, kecuali pengguna menghentikan server.
                if db_status(&state.pool, server_id).await.as_deref() == Some("error") {
                    let _ = sqlx::query(
                        "UPDATE server_instances SET status='degraded', error_message=?1, updated_at=?2 WHERE id=?3",
                    )
                    .bind(&msg)
                    .bind(now_iso())
                    .bind(server_id)
                    .execute(&state.pool)
                    .await;
                }
            }
        }
    }
}

/// Supervisi kesehatan berkelanjutan: tiap 10 detik probe HTTP nyata ke
/// alamat bind sebenarnya untuk server running/degraded. Transisi
/// running → degraded → restart otomatis (maks 3) → error semuanya
/// tercatat di log dan kolom kesehatan server.
pub fn spawn_health_supervisor(state: AppState) {
    tokio::spawn(async move {
        let mut healthy_streaks: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let rows = match sqlx::query(
                "SELECT s.id FROM server_instances s WHERE s.status IN ('running','degraded') ORDER BY s.created_at ASC",
            )
            .fetch_all(&state.pool)
            .await
            {
                Ok(rows) => rows,
                Err(_) => continue,
            };
            use sqlx::Row;
            for row in rows {
                let server_id: String = row.get("id");
                if state.health_restarting.lock().await.contains(&server_id) {
                    continue;
                }
                let (server, project_path) =
                    match server_row_for_supervisor(&state.pool, &server_id).await {
                        Some(v) => v,
                        None => continue,
                    };
                if !matches!(server.status.as_str(), "running" | "degraded") {
                    continue;
                }
                let target = probe_target_for(server.bind_lan, server.port as u16);
                let probe = probe_http_target(&target, Duration::from_millis(2500)).await;
                let doc_root = crate::util::resolve_inside(
                    &PathBuf::from(&project_path),
                    &server.document_root,
                    true,
                )
                .unwrap_or_else(|_| PathBuf::from(&project_path));
                let effective_proxy = server
                    .proxy_port
                    .and_then(|p| u16::try_from(p).ok())
                    .or_else(|| server.backend_port.and_then(|p| u16::try_from(p).ok()));
                let warning = probe_warning(
                    &probe,
                    docroot_warning(
                        &server.engine,
                        &PathBuf::from(&project_path),
                        &doc_root,
                        effective_proxy,
                    ),
                );
                let transition = next_health_transition(
                    &server.status,
                    server.health_failures,
                    probe.class(),
                    server.auto_restart_count,
                );
                update_health_fields(
                    &state.pool,
                    &server.id,
                    &probe,
                    transition.failures,
                    warning.as_deref(),
                )
                .await;
                if probe.class().is_alive() {
                    let streak = healthy_streaks.entry(server.id.clone()).or_insert(0);
                    *streak += 1;
                    if *streak >= 3 && server.auto_restart_count != 0 {
                        let _ = sqlx::query(
                            "UPDATE server_instances SET auto_restart_count=0, updated_at=?1 WHERE id=?2",
                        )
                        .bind(now_iso())
                        .bind(&server.id)
                        .execute(&state.pool)
                        .await;
                    }
                    if server.status == "degraded" {
                        let _ = sqlx::query(
                            "UPDATE server_instances SET status='running', error_message=NULL, updated_at=?1 WHERE id=?2",
                        )
                        .bind(now_iso())
                        .bind(&server.id)
                        .execute(&state.pool)
                        .await;
                        app_log(
                            &state.pool,
                            "server",
                            "info",
                            &format!(
                                "Supervisi kesehatan: server pulih dan menjawab HTTP lagi. {}",
                                probe.evidence()
                            ),
                            Some(&server.id),
                        )
                        .await;
                    }
                    continue;
                }
                healthy_streaks.insert(server.id.clone(), 0);
                let evidence = probe.evidence();
                match transition.action {
                    HealthAction::None => {
                        if transition.failures == 1 {
                            app_log(
                                &state.pool,
                                "server",
                                "warn",
                                &format!(
                                    "Supervisi kesehatan: probe gagal (1x). {}",
                                    evidence
                                ),
                                Some(&server.id),
                            )
                            .await;
                        }
                    }
                    HealthAction::MarkDegraded => {
                        let msg = format!(
                            "Server degraded: probe HTTP gagal beruntun. {}",
                            evidence
                        );
                        let _ = sqlx::query(
                            "UPDATE server_instances SET status='degraded', error_message=?1, updated_at=?2 WHERE id=?3",
                        )
                        .bind(&msg)
                        .bind(now_iso())
                        .bind(&server.id)
                        .execute(&state.pool)
                        .await;
                        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                    }
                    HealthAction::Restart => {
                        let liveness = process_liveness(&state, &server).await;
                        if liveness == ProcessLiveness::ExitedHandled {
                            app_log(
                                &state.pool,
                                "server",
                                "warn",
                                &format!(
                                    "Supervisi kesehatan: proses keluar dan watcher engine sedang menangani restart. {}",
                                    evidence
                                ),
                                Some(&server.id),
                            )
                            .await;
                            continue;
                        }
                        let msg = format!(
                            "Server degraded: probe HTTP gagal beruntun dan restart otomatis dijadwalkan. {}",
                            evidence
                        );
                        let _ = sqlx::query(
                            "UPDATE server_instances SET status='degraded', error_message=?1, updated_at=?2 WHERE id=?3",
                        )
                        .bind(&msg)
                        .bind(now_iso())
                        .bind(&server.id)
                        .execute(&state.pool)
                        .await;
                        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                        spawn_supervisor_restart(state.clone(), server.id.clone(), evidence);
                    }
                    HealthAction::MarkError => {
                        let msg = format!(
                            "Server error: tidak menjawab HTTP dan anggaran restart otomatis habis. {}",
                            evidence
                        );
                        stop_process_internal(&state, &server.id).await;
                        set_status(
                            &state.pool,
                            &server.id,
                            "error",
                            None,
                            None,
                            Some(&msg),
                            false,
                        )
                        .await;
                        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                    }
                }
            }
        }
    });
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
            if !matches!(status.as_deref(), Some("running") | Some("degraded") | Some("starting")) {
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
                        let probe = wait_server_healthy(&server, Duration::from_secs(5)).await;
                        if probe.ok {
                            set_running_with_probe(
                                &state.pool,
                                &server.id,
                                pid,
                                false,
                                &probe,
                                None,
                                false,
                            )
                            .await;
                            app_log(
                                &state.pool,
                                "server",
                                "info",
                                &format!(
                                    "Restart otomatis berhasil (percobaan {}), health check lolos. {}",
                                    attempts_used,
                                    probe.evidence()
                                ),
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
    project_path: PathBuf,
) -> tokio::task::AbortHandle {
    let port = server.port as u16;
    let handle = tokio::spawn(async move {
        let mut attempts_used: u32 = 0;
        loop {
            let listener = match tokio::net::TcpListener::bind((host.as_str(), port)).await {
                Ok(l) => l,
                Err(e) => {
                    let status = db_status(&state.pool, &server.id).await;
                    if !matches!(status.as_deref(), Some("running") | Some("degraded") | Some("starting")) {
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
            // Probe kesehatan berjalan di task terpisah dan mengirim
            // hasilnya lewat channel. Meng-await probe di cabang select
            // akan menghentikan polling future serve, sehingga server
            // tidak pernah menjawab probe-nya sendiri dan health
            // deadline memicu restart loop (bug v0.4.0).
            let (probe_tx, mut probe_rx) = tokio::sync::mpsc::channel::<HttpProbe>(4);
            let probe_target = probe_target_for(server.bind_lan, port);
            let probe_task = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let probe =
                        probe_http_target(&probe_target, Duration::from_millis(1600)).await;
                    let alive = probe.class().is_alive();
                    if probe_tx.send(probe).await.is_err() || alive {
                        return;
                    }
                }
            });
            let health_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            let serve_result = loop {
                tokio::select! {
                    r = &mut serve => break r.map_err(|e| e.to_string()),
                    probe = probe_rx.recv() => {
                        match probe {
                            Some(probe) if probe.class().is_alive() => {
                                let status = db_status(&state.pool, &server.id).await;
                                if status.as_deref() == Some("starting") {
                                    let warning = probe_warning(
                                        &probe,
                                        docroot_warning("static", &project_path, &doc_root, None),
                                    );
                                    set_running_with_probe(
                                        &state.pool,
                                        &server.id,
                                        None,
                                        true,
                                        &probe,
                                        warning.as_deref(),
                                        attempts_used == 0,
                                    )
                                    .await;
                                    app_log(
                                        &state.pool,
                                        "server",
                                        "info",
                                        &format!(
                                            "Health check lolos, server berjalan di port {}. {}",
                                            port,
                                            probe.evidence()
                                        ),
                                        Some(&server.id),
                                    )
                                    .await;
                                }
                                // Sehat: lanjut melayani sampai serve berakhir.
                                break serve.await.map_err(|e| e.to_string());
                            }
                            Some(probe) => {
                                if tokio::time::Instant::now() > health_deadline {
                                    break Err(format!(
                                        "Health check gagal dalam 5 detik. {}",
                                        probe.evidence()
                                    ));
                                }
                            }
                            None => {
                                if tokio::time::Instant::now() > health_deadline {
                                    break Err("Health check gagal dalam 5 detik".to_string());
                                }
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(health_deadline) => {
                        break Err("Health check gagal dalam 5 detik".to_string());
                    }
                }
            };
            probe_task.abort();
            // Task serve berakhir (atau health check gagal).
            let status = db_status(&state.pool, &server.id).await;
            if !matches!(status.as_deref(), Some("running") | Some("degraded") | Some("starting")) {
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
    project_path: &str,
    doc_root: &std::path::Path,
    reset_restart_budget: bool,
) -> Result<(), String> {
    let (pid, child_arc) = match launch_child(state, server, &launch).await {
        Ok(v) => v,
        Err(msg) => return Err(fail_start(state, server, msg).await),
    };
    spawn_child_watcher(state.clone(), server.clone(), launch, child_arc);
    start_log_tails(state, server);
    finish_start(
        state,
        server,
        pid,
        false,
        project_path,
        doc_root,
        reset_restart_budget,
    )
    .await
}

fn free_loopback_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|addr| addr.port())
}

/// Port proxy generik efektif: hanya proxy eksplisit pengguna.
/// Komposit PHP memakai [`php_fpm_backend_port`] yang terpisah.
pub(crate) fn effective_proxy_port(server: &ServerRow) -> Option<u16> {
    server.proxy_port.and_then(|p| u16::try_from(p).ok())
}

/// true bila server ini mode komposit PHP-FPM: engine web server
/// (Nginx/Caddy/Apache), proyek PHP, dan tanpa proxy eksplisit.
pub(crate) fn is_fpm_composite(
    server: &ServerRow,
    project_path: &std::path::Path,
    doc_root: &std::path::Path,
) -> bool {
    matches!(server.engine.as_str(), "nginx" | "caddy" | "apache")
        && server.proxy_port.is_none()
        && is_php_project(project_path, doc_root)
}

/// Port backend PHP-FPM untuk mode komposit (dialokasikan dan disimpan
/// ke backend_port bila `allocate` true). None bila bukan komposit.
pub(crate) async fn php_fpm_backend_port(
    state: &AppState,
    server: &ServerRow,
    project_path: &std::path::Path,
    doc_root: &std::path::Path,
    allocate: bool,
) -> Option<u16> {
    if !is_fpm_composite(server, project_path, doc_root) {
        return None;
    }
    if let Some(port) = server.backend_port.and_then(|p| u16::try_from(p).ok()) {
        if port != server.port as u16 && port_bind_test("127.0.0.1", port) {
            return Some(port);
        }
        if !allocate {
            return Some(port);
        }
    }
    if !allocate {
        return server.backend_port.and_then(|p| u16::try_from(p).ok());
    }
    let port = free_loopback_port()?;
    let _ = sqlx::query(
        "UPDATE server_instances SET backend_port=?1, updated_at=?2 WHERE id=?3",
    )
    .bind(port as i64)
    .bind(now_iso())
    .bind(&server.id)
    .execute(&state.pool)
    .await;
    Some(port)
}

/// backend_kind yang disimpan untuk server: "php-fpm" untuk komposit
/// FPM, "proxy" untuk proxy eksplisit, None selain itu.
pub(crate) fn backend_kind_for(server: &ServerRow, fpm_port: Option<u16>) -> Option<&'static str> {
    if fpm_port.is_some() {
        Some("php-fpm")
    } else if server.proxy_port.is_some() {
        Some("proxy")
    } else {
        None
    }
}

async fn set_backend_kind(pool: &sqlx::SqlitePool, server_id: &str, kind: Option<&str>) {
    let _ = sqlx::query(
        "UPDATE server_instances SET backend_kind=?1, updated_at=?2 WHERE id=?3",
    )
    .bind(kind)
    .bind(now_iso())
    .bind(server_id)
    .execute(pool)
    .await;
}

/// Tunggu port TCP loopback menerima koneksi (kesiapan backend FPM).
async fn wait_tcp_ready(port: u16, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Ekor file log (baris terakhir) untuk pesan error jujur.
fn file_tail(path: &std::path::Path, lines: usize) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .rev()
            .take(lines)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        Err(_) => Vec::new(),
    }
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
    let proxy_port = effective_proxy_port(server);
    let php_fpm_port = php_fpm_backend_port(
        state,
        server,
        &PathBuf::from(project_path),
        &doc_root,
        true,
    )
    .await;
    let input = crate::webservers::WebServerInput {
        host: if server.bind_lan { "0.0.0.0".to_string() } else { "127.0.0.1".to_string() },
        port: server.port as u16,
        doc_root,
        project_path: PathBuf::from(project_path),
        data_dir: state.config.data_dir.clone(),
        server_id: server.id.clone(),
        proxy_port,
        php_fpm_port,
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
    start_server_inner(state, server, project_path, true).await
}

async fn start_server_inner(
    state: &AppState,
    server: &ServerRow,
    project_path: &str,
    reset_restart_budget: bool,
) -> Result<(), String> {
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
        "php" => match php::resolve_php(
            state.native_lib_dir.as_deref(),
            server.effective_php_version(),
        )
        .await
        {
            Ok(resolved) => Some(resolved.binary),
            Err(msg) => {
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                return Err(msg);
            }
        },
        "caddy" | "nginx" | "apache" | "python" => match find_engine_binary(state, engine) {
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

    // Mode komposit PHP-FPM: versi PHP efektif wajib tersedia beserta
    // binary FPM-nya sebelum port dialokasikan.
    let mut resolved_fpm: Option<php::ResolvedPhp> = None;
    if is_fpm_composite(server, &PathBuf::from(project_path), &doc_root) {
        match php::resolve_php(
            state.native_lib_dir.as_deref(),
            server.effective_php_version(),
        )
        .await
        {
            Ok(resolved) => {
                if resolved.fpm_binary.is_none() {
                    let spec = php::php_version_spec(&resolved.version);
                    let msg = format!(
                        "Mode komposit PHP-FPM tidak dapat dimulai: binary PHP-FPM {} tidak ditemukan ({} di nativeLibraryDir atau php-fpm di PATH)",
                        resolved.version,
                        spec.map(|s| s.fpm_binary).unwrap_or("libphpfpm.so")
                    );
                    set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                    app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                    return Err(msg);
                }
                resolved_fpm = Some(resolved);
            }
            Err(msg) => {
                set_status(&state.pool, &server.id, "error", None, None, Some(&msg), false).await;
                app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
                return Err(msg);
            }
        }
    }

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
            let smtp = crate::routes::mail::load_smtp_config(&state.pool, &state.config.data_dir).await;
            let ini_path = match php::write_php_ini_with_smtp(
                &state.config.data_dir,
                &settings,
                smtp.as_ref(),
            ) {
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
            launch_and_finish(state, server, launch, project_path, &doc_root, reset_restart_budget).await
        }
        "caddy" | "nginx" | "apache" => {
            let binary = binary.expect("webserver binary validated");
            let effective_proxy = effective_proxy_port(server);
            let fpm_port = php_fpm_backend_port(
                state,
                server,
                &PathBuf::from(project_path),
                &doc_root,
                true,
            )
            .await;
            let mut effective_server = server.clone();
            if fpm_port.is_some() {
                effective_server.backend_port = fpm_port.map(|p| p as i64);
            }
            set_backend_kind(
                &state.pool,
                &server.id,
                backend_kind_for(server, fpm_port),
            )
            .await;
            let input = crate::webservers::WebServerInput {
                host: host.to_string(),
                port: port_u16,
                doc_root: doc_root.clone(),
                project_path: PathBuf::from(project_path),
                data_dir: state.config.data_dir.clone(),
                server_id: server.id.clone(),
                proxy_port: effective_proxy,
                php_fpm_port: fpm_port,
                custom_config: server.config_text.clone(),
            };
            let config_path = match crate::webservers::write_config(&input, engine) {
                Ok(p) => p,
                Err(e) => return Err(fail_start(state, server, format!("Gagal menulis konfigurasi: {}", e)).await),
            };
            let home = crate::webservers::server_home(&state.config.data_dir, &server.id, engine);
            let _ = std::fs::create_dir_all(home.join("tmp"));
            // Validasi konfigurasi dengan binary sungguhan sebelum start.
            match validate_webserver_config(state, &effective_server, project_path, None).await {
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
            if let Some(backend_port) = fpm_port {
                // Mode komposit PHP-FPM: FPM di loopback backend_port,
                // frontend meneruskan file .php ke FPM (fastcgi/fcgi).
                let resolved = match resolved_fpm.clone() {
                    Some(r) => r,
                    None => {
                        return Err(fail_start(
                            state,
                            server,
                            "Mode komposit PHP-FPM gagal: resolusi PHP tidak tersedia".to_string(),
                        )
                        .await)
                    }
                };
                let fpm_binary = match resolved.fpm_binary.clone() {
                    Some(b) => b,
                    None => {
                        return Err(fail_start(
                            state,
                            server,
                            format!(
                                "Mode komposit PHP-FPM gagal: binary PHP-FPM {} tidak ditemukan",
                                resolved.version
                            ),
                        )
                        .await)
                    }
                };
                let settings = php::load_php_settings(&state.pool).await;
                let smtp = crate::routes::mail::load_smtp_config(&state.pool, &state.config.data_dir).await;
                if let Err(e) = php::write_php_ini_with_smtp(
                    &state.config.data_dir,
                    &settings,
                    smtp.as_ref(),
                ) {
                    return Err(fail_start(
                        state,
                        server,
                        format!("Gagal menulis php.ini backend komposit: {}", e),
                    )
                    .await);
                }
                let fpm_input = php::FpmConfigInput {
                    data_dir: state.config.data_dir.clone(),
                    server_id: server.id.clone(),
                    backend_port,
                    native_lib_dir: state.native_lib_dir.clone(),
                };
                let fpm_config = match php::write_fpm_config(&fpm_input) {
                    Ok(p) => p,
                    Err(e) => {
                        return Err(fail_start(
                            state,
                            server,
                            format!("Gagal menulis php-fpm.conf: {}", e),
                        )
                        .await)
                    }
                };
                // Validasi FPM sebelum start; kegagalan membawa output
                // validasi apa adanya.
                let fpm_home = php::fpm_home(&state.config.data_dir, &server.id);
                let mut validate_cmd = tokio::process::Command::new(&fpm_binary);
                validate_cmd.args(php::fpm_validate_args(&fpm_config));
                crate::util::prepare_spawn(
                    &mut validate_cmd,
                    state.native_lib_dir.as_deref(),
                    &fpm_home,
                    Some(&fpm_home),
                );
                for (k, v) in php::fpm_envs(&state.config.data_dir) {
                    validate_cmd.env(k, v);
                }
                match tokio::time::timeout(Duration::from_secs(10), validate_cmd.output()).await {
                    Ok(Ok(out)) if out.status.success() => {}
                    Ok(Ok(out)) => {
                        let output = format!(
                            "{}{}",
                            String::from_utf8_lossy(&out.stdout),
                            String::from_utf8_lossy(&out.stderr)
                        );
                        return Err(fail_start(
                            state,
                            server,
                            format!(
                                "Konfigurasi PHP-FPM {} tidak valid: {}",
                                resolved.version,
                                output.trim()
                            ),
                        )
                        .await);
                    }
                    Ok(Err(e)) => {
                        return Err(fail_start(
                            state,
                            server,
                            format!("Validasi PHP-FPM gagal dijalankan: {}", e),
                        )
                        .await)
                    }
                    Err(_) => {
                        return Err(fail_start(
                            state,
                            server,
                            "Validasi PHP-FPM timeout".to_string(),
                        )
                        .await)
                    }
                }
                let backend_launch = ChildLaunch {
                    program: fpm_binary,
                    args: php::fpm_launch_args(&fpm_config),
                    cwd: Some(PathBuf::from(project_path)),
                    envs: php::fpm_envs(&state.config.data_dir),
                    label: format!("PHP-FPM {} backend komposit", resolved.version),
                };
                let (_backend_pid, backend_arc) =
                    match spawn_child_arc(state, &effective_server, &backend_launch, true).await {
                        Ok(v) => v,
                        Err(msg) => return Err(fail_start(state, server, msg).await),
                    };
                // Kesiapan backend: TCP FPM menerima koneksi.
                if !wait_tcp_ready(backend_port, Duration::from_secs(5)).await {
                    let mut tail = stderr_tail(state, &server.id, 10).await;
                    tail.extend(file_tail(
                        &php::fpm_error_log_path(&state.config.data_dir, &server.id),
                        10,
                    ));
                    kill_child_arc(&backend_arc).await;
                    let msg = format!(
                        "Backend PHP-FPM {} gagal siap di 127.0.0.1:{} dalam 5 detik. Stderr/log terakhir:\n{}",
                        resolved.version,
                        backend_port,
                        if tail.is_empty() {
                            "proses tidak mengeluarkan output".to_string()
                        } else {
                            tail.join("\n")
                        }
                    );
                    return Err(fail_start(state, server, msg).await);
                }
                app_log(
                    &state.pool,
                    "server",
                    "info",
                    &format!(
                        "Mode komposit PHP-FPM aktif: {} meneruskan file .php ke PHP-FPM {} di 127.0.0.1:{}",
                        engine, resolved.version, backend_port
                    ),
                    Some(&server.id),
                )
                .await;
                let (frontend_pid, frontend_arc) =
                    match spawn_child_arc(state, &effective_server, &launch, false).await {
                        Ok(v) => v,
                        Err(msg) => {
                            kill_child_arc(&backend_arc).await;
                            return Err(fail_start(state, server, msg).await);
                        }
                    };
                state.processes.lock().await.insert(
                    server.id.clone(),
                    ProcessHandle::Composite {
                        frontend: frontend_arc,
                        backend: backend_arc,
                    },
                );
                start_log_tails(state, &effective_server);
                finish_start(
                    state,
                    &effective_server,
                    frontend_pid,
                    false,
                    project_path,
                    &doc_root,
                    reset_restart_budget,
                )
                .await
            } else {
                launch_and_finish(
                    state,
                    &effective_server,
                    launch,
                    project_path,
                    &doc_root,
                    reset_restart_budget,
                )
                .await
            }
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
            launch_and_finish(state, server, launch, project_path, &doc_root, reset_restart_budget).await
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
                start_node_android(
                    state,
                    server,
                    &project,
                    args,
                    project_path,
                    &doc_root,
                    reset_restart_budget,
                )
                .await
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
                launch_and_finish(state, server, launch, project_path, &doc_root, reset_restart_budget).await
            }
        }
        _ => {
            // static
            #[cfg(target_os = "android")]
            {
                let abort = spawn_inprocess_supervisor(
                    state.clone(),
                    server.clone(),
                    host.to_string(),
                    doc_root.clone(),
                    PathBuf::from(project_path),
                );
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
                finish_start(
                    state,
                    server,
                    None,
                    true,
                    project_path,
                    &doc_root,
                    reset_restart_budget,
                )
                .await
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
                launch_and_finish(state, server, launch, project_path, &doc_root, reset_restart_budget).await
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
    project_path: &str,
    doc_root: &std::path::Path,
    reset_restart_budget: bool,
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
    finish_start(
        state,
        server,
        None,
        false,
        project_path,
        doc_root,
        reset_restart_budget,
    )
    .await
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
            if !matches!(status.as_deref(), Some("running") | Some("degraded") | Some("starting")) {
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
                        let probe = if ok {
                            wait_server_healthy(&server, Duration::from_secs(15)).await
                        } else {
                            probe_http_target(
                                &probe_target_for(server.bind_lan, server.port as u16),
                                Duration::from_millis(100),
                            )
                            .await
                        };
                        if ok && probe.ok {
                            set_running_with_probe(
                                &state.pool,
                                &server.id,
                                None,
                                false,
                                &probe,
                                None,
                                false,
                            )
                            .await;
                            app_log(
                                &state.pool,
                                "server",
                                "info",
                                &format!(
                                    "Restart Node berhasil, health check lolos. {}",
                                    probe.evidence()
                                ),
                                Some(&server.id),
                            )
                            .await;
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

async fn finish_start(
    state: &AppState,
    server: &ServerRow,
    pid: Option<i64>,
    in_process: bool,
    project_path: &str,
    doc_root: &std::path::Path,
    reset_restart_budget: bool,
) -> Result<(), String> {
    let timeout = engine_health_timeout(&server.engine);
    let probe = wait_server_healthy(server, timeout).await;
    if probe.class().is_alive() {
        let effective_proxy = server
            .proxy_port
            .and_then(|p| u16::try_from(p).ok())
            .or_else(|| server.backend_port.and_then(|p| u16::try_from(p).ok()));
        let warning = probe_warning(
            &probe,
            docroot_warning(
                &server.engine,
                &PathBuf::from(project_path),
                doc_root,
                effective_proxy,
            ),
        );
        set_running_with_probe(
            &state.pool,
            &server.id,
            pid,
            in_process,
            &probe,
            warning.as_deref(),
            reset_restart_budget,
        )
        .await;
        app_log(
            &state.pool,
            "server",
            "info",
            &format!(
                "Health check lolos, server berjalan di port {}. {}",
                server.port,
                probe.evidence()
            ),
            Some(&server.id),
        )
        .await;
        if let Some(warning) = warning {
            app_log(&state.pool, "server", "warn", &warning, Some(&server.id)).await;
        }
        Ok(())
    } else {
        // Ambil ekor stderr sebelum proses dihentikan agar error_message
        // memuat penyebab nyata dari proses, bukan pesan generik.
        let tail = stderr_tail(state, &server.id, 10).await;
        let msg = format!(
            "{} Bukti probe terakhir: {}",
            compose_health_error(timeout.as_secs(), &tail),
            probe.evidence()
        );
        update_health_fields(&state.pool, &server.id, &probe, 1, None).await;
        stop_process_internal(state, &server.id).await;
        set_status(&state.pool, &server.id, "error", None, None, Some(&msg), in_process).await;
        app_log(&state.pool, "server", "error", &msg, Some(&server.id)).await;
        Err(msg)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LanTestResult {
    pub ok: bool,
    pub url: Option<String>,
    pub host: Option<String>,
    pub port: i64,
    pub status_code: Option<u16>,
    pub duration_ms: u64,
    pub target: Option<String>,
    pub error: Option<String>,
    pub warning: Option<String>,
}

/// Tes Akses LAN nyata dari core: HTTP GET ke IP LAN:port dengan Host LAN.
pub async fn test_lan_access(state: &AppState, server: &ServerRow) -> LanTestResult {
    let _ = state;
    if !matches!(server.status.as_str(), "running" | "degraded") {
        return LanTestResult {
            ok: false,
            url: None,
            host: None,
            port: server.port,
            status_code: None,
            duration_ms: 0,
            target: None,
            error: Some(format!(
                "Server tidak berjalan (status {}); Tes Akses LAN hanya untuk server running/degraded",
                server.status
            )),
            warning: None,
        };
    }
    if !server.bind_lan {
        return LanTestResult {
            ok: false,
            url: None,
            host: None,
            port: server.port,
            status_code: None,
            duration_ms: 0,
            target: None,
            error: Some("Akses LAN tidak dibuka untuk server ini (bind_lan nonaktif)".to_string()),
            warning: None,
        };
    }
    let Some(ip) = crate::network::lan_ip() else {
        return LanTestResult {
            ok: false,
            url: None,
            host: None,
            port: server.port,
            status_code: None,
            duration_ms: 0,
            target: None,
            error: Some("IP LAN perangkat tidak terdeteksi".to_string()),
            warning: None,
        };
    };
    let target = probe_target_with_lan(true, server.port as u16, Some(ip.clone()));
    let probe = probe_http_target(&target, Duration::from_secs(3)).await;
    let warning = match probe.status_code {
        Some(403) => Some(
            "Server menjawab HTTP 403 pada / lewat LAN; periksa index document root dan izin folder proyek"
                .to_string(),
        ),
        Some(404) => Some(
            "Server menjawab HTTP 404 pada / lewat LAN; periksa document root dan index proyek"
                .to_string(),
        ),
        _ => None,
    };
    LanTestResult {
        ok: probe.ok,
        url: Some(target.label.clone()),
        host: Some(ip),
        port: server.port,
        status_code: probe.status_code,
        duration_ms: probe.duration_ms,
        target: Some(probe.target.clone()),
        error: probe.error.clone(),
        warning,
    }
}

async fn stop_process_internal(state: &AppState, server_id: &str) {
    stop_log_tails(state, server_id).await;
    let handle = state.processes.lock().await.remove(server_id);
    match handle {
        Some(ProcessHandle::Child(arc)) => {
            kill_child_arc(&arc).await;
        }
        Some(ProcessHandle::Composite { frontend, backend }) => {
            kill_child_arc(&frontend).await;
            kill_child_arc(&backend).await;
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
    set_backend_kind(&state.pool, server_id, None).await;
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

    fn probe_with_status(status_code: Option<u16>) -> HttpProbe {
        HttpProbe {
            ok: status_code.is_some_and(|code| code < 500),
            reachable: status_code.is_some(),
            status_code,
            duration_ms: 7,
            target: "http://192.168.1.20:8080/".to_string(),
            error: None,
        }
    }

    #[test]
    fn lan_probe_target_uses_lan_host_header() {
        let target = probe_target_with_lan(true, 8080, Some("192.168.1.20".to_string()));
        assert_eq!(target.host, "192.168.1.20");
        assert_eq!(target.host_header, "192.168.1.20:8080");
        assert!(target.fallback_note.is_none());

        let fallback = probe_target_with_lan(true, 8080, None);
        assert_eq!(fallback.host, "127.0.0.1");
        assert!(fallback.fallback_note.unwrap().contains("IP LAN"));
    }

    #[test]
    fn probe_classification() {
        assert_eq!(probe_with_status(Some(200)).class(), ProbeClass::Alive);
        assert_eq!(probe_with_status(Some(403)).class(), ProbeClass::Alive);
        assert_eq!(probe_with_status(Some(404)).class(), ProbeClass::Alive);
        assert_eq!(probe_with_status(Some(301)).class(), ProbeClass::Alive);
        assert_eq!(probe_with_status(Some(500)).class(), ProbeClass::ServerError);
        assert_eq!(probe_with_status(Some(502)).class(), ProbeClass::ServerError);
        assert_eq!(probe_with_status(None).class(), ProbeClass::TransportFailure);
    }

    #[test]
    fn health_transition_404_and_403_never_restart() {
        // 404/403 beruntun: server hidup, failures selalu reset.
        for code in [404u16, 403] {
            let probe = probe_with_status(Some(code));
            let mut failures = 0;
            for _ in 0..5 {
                let t = next_health_transition("running", failures, probe.class(), 0);
                assert_eq!(t.failures, 0, "HTTP {} tidak boleh menambah failures", code);
                assert_eq!(t.status, "running");
                assert_eq!(t.action, HealthAction::None);
                failures = t.failures;
            }
        }
        // Bahkan dari degraded dengan failures menumpuk, 404 memulihkan.
        let t = next_health_transition(
            "degraded",
            2,
            probe_with_status(Some(404)).class(),
            2,
        );
        assert_eq!(t.failures, 0);
        assert_eq!(t.status, "running");
        assert_eq!(t.action, HealthAction::None);
    }

    #[test]
    fn health_transition_500_and_transport_restart() {
        for class in [ProbeClass::ServerError, ProbeClass::TransportFailure] {
            let first = next_health_transition("running", 0, class, 0);
            assert_eq!(first.action, HealthAction::None);
            let second = next_health_transition("running", first.failures, class, 0);
            assert_eq!(second.action, HealthAction::MarkDegraded);
            let third = next_health_transition("degraded", second.failures, class, 0);
            assert_eq!(third.action, HealthAction::Restart);
            let exhausted = next_health_transition(
                "degraded",
                2,
                class,
                php::MAX_RESTART_ATTEMPTS as i64,
            );
            assert_eq!(exhausted.action, HealthAction::MarkError);
        }
    }

    #[test]
    fn health_transition_degrades_then_restarts_then_errors() {
        let first = next_health_transition("running", 0, ProbeClass::TransportFailure, 0);
        assert_eq!(first.failures, 1);
        assert_eq!(first.status, "running");
        assert_eq!(first.action, HealthAction::None);

        let second = next_health_transition("running", first.failures, ProbeClass::TransportFailure, 0);
        assert_eq!(second.failures, 2);
        assert_eq!(second.status, "degraded");
        assert_eq!(second.action, HealthAction::MarkDegraded);

        let third = next_health_transition("degraded", second.failures, ProbeClass::TransportFailure, 0);
        assert_eq!(third.failures, 3);
        assert_eq!(third.status, "degraded");
        assert_eq!(third.action, HealthAction::Restart);

        let exhausted = next_health_transition(
            "degraded",
            2,
            ProbeClass::TransportFailure,
            php::MAX_RESTART_ATTEMPTS as i64,
        );
        assert_eq!(exhausted.status, "error");
        assert_eq!(exhausted.action, HealthAction::MarkError);

        let recovered = next_health_transition("degraded", 2, ProbeClass::Alive, 1);
        assert_eq!(recovered.failures, 0);
        assert_eq!(recovered.status, "running");
        assert_eq!(recovered.action, HealthAction::None);
    }

    #[test]
    fn probe_403_is_warning_not_full_health() {
        let probe = probe_with_status(Some(403));
        assert!(probe.ok);
        let warning = probe_warning(&probe, None).unwrap();
        assert!(warning.contains("HTTP 403"));
        assert!(warning.contains("index"));

        let healthy = probe_with_status(Some(200));
        assert!(probe_warning(&healthy, None).is_none());
    }

    #[test]
    fn docroot_warning_detects_php_without_index_html() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "wf-docroot-warning-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.php"), "<?php echo 'ok';").unwrap();

        let warning = docroot_warning("nginx", &root, &root, None).unwrap();
        assert!(warning.contains("index.php"));
        assert!(warning.contains("PHP-FPM"));
        assert!(docroot_warning("nginx", &root, &root, Some(9000)).is_none());
        // Apache+PHP kini didukung lewat FPM: tanpa backend aktif
        // peringatannya mengarahkan ke mode komposit, bukan penolakan.
        let apache = docroot_warning("apache", &root, &root, None).unwrap();
        assert!(apache.contains("PHP-FPM"));
        assert!(docroot_warning("apache", &root, &root, Some(19001)).is_none());
        let statik = docroot_warning("static", &root, &root, None).unwrap();
        assert!(statik.contains("tidak mengeksekusi PHP"));
        assert!(docroot_warning("php", &root, &root, None).is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn log_tail_chunk_parses_complete_lines_and_severity() {
        let mut buffer = String::new();
        let lines = parse_log_tail_chunk(&mut buffer, b"baris satu\nbaris dua");
        assert_eq!(lines, vec!["baris satu".to_string()]);
        assert_eq!(buffer, "baris dua");

        assert_eq!(
            nginx_log_severity("2026/01/01 10:00:00 [error] 1#1: gagal"),
            "error"
        );
        assert_eq!(
            apache_log_severity("[Mon Jan 01 10:00:00 2026] [core:warn] [pid 1] peringatan"),
            "warn"
        );
        assert_eq!(
            apache_log_severity("[Mon Jan 01 10:00:00 2026] [core:error] [pid 1] gagal"),
            "error"
        );
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
