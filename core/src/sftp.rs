//! Server SFTP WebForge Mobile (russh + russh-sftp).
//!
//! Keamanan:
//! - Autentikasi hanya password; password disimpan sebagai hash Argon2id
//!   di tabel `sftp_settings`, tidak pernah dikembalikan API/log.
//! - Root SFTP adalah `workspace_dir` aplikasi. Semua path virtual "/"
//!   dipetakan ke workspace; normalisasi menolak `..` yang keluar root
//!   dan symlink yang target ter-canonicalize-nya keluar workspace.
//! - Host key ed25519 persisten di `<data_dir>/sftp/host_key` (mode 0600
//!   di Unix), dibuat sekali; fingerprint SHA256 ditampilkan di status.
//! - Bind default 127.0.0.1; `bind_lan` mengubahnya ke 0.0.0.0.
//!
//! Koneksi aktif dilacak in-memory. Aktivitas koneksi/auth/start/stop
//! dicatat ke `application_logs` kategori "sftp" tanpa password.

use crate::state::app_log;
use crate::util::{iso_of, new_id, now_iso};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Mutex};

pub const DEFAULT_USERNAME: &str = "webforge";
pub const DEFAULT_PORT: u16 = 2222;
const READDIR_BATCH: usize = 100;
const MAX_READ_LEN: u32 = 1024 * 1024;

// ---------------------------------------------------------------------------
// Pengaturan & kredensial
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SftpSettings {
    pub username: String,
    pub password_hash: Option<String>,
    pub port: u16,
    pub bind_lan: bool,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            username: DEFAULT_USERNAME.to_string(),
            password_hash: None,
            port: DEFAULT_PORT,
            bind_lan: false,
        }
    }
}

pub fn validate_settings(username: &str, port: u16) -> Result<(), String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("Username SFTP tidak boleh kosong".to_string());
    }
    if username.len() > 64 {
        return Err("Username SFTP maksimal 64 karakter".to_string());
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(
            "Username SFTP hanya boleh berisi huruf, angka, titik, garis bawah, dan tanda hubung"
                .to_string(),
        );
    }
    if !(1024..=65535).contains(&port) {
        return Err("Port SFTP harus antara 1024 dan 65535".to_string());
    }
    Ok(())
}

pub async fn load_settings(pool: &SqlitePool) -> Result<SftpSettings, String> {
    let row = sqlx::query(
        "SELECT username, password_hash, port, bind_lan FROM sftp_settings WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Gagal membaca pengaturan SFTP: {}", e))?;
    match row {
        Some(r) => {
            let port_i: Option<i64> = r.get("port");
            let port = port_i
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(DEFAULT_PORT);
            Ok(SftpSettings {
                username: r
                    .get::<Option<String>, _>("username")
                    .filter(|u| !u.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_USERNAME.to_string()),
                password_hash: r.get::<Option<String>, _>("password_hash"),
                port,
                bind_lan: r.get::<Option<i64>, _>("bind_lan").unwrap_or(0) != 0,
            })
        }
        None => Ok(SftpSettings::default()),
    }
}

pub async fn save_settings(pool: &SqlitePool, settings: &SftpSettings) -> Result<(), String> {
    let now = now_iso();
    sqlx::query(
        "INSERT INTO sftp_settings (id, username, password_hash, port, bind_lan, created_at, updated_at) \
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?5) \
         ON CONFLICT(id) DO UPDATE SET username = excluded.username, \
         password_hash = excluded.password_hash, port = excluded.port, \
         bind_lan = excluded.bind_lan, updated_at = excluded.updated_at",
    )
    .bind(&settings.username)
    .bind(&settings.password_hash)
    .bind(settings.port as i64)
    .bind(if settings.bind_lan { 1i64 } else { 0i64 })
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| format!("Gagal menyimpan pengaturan SFTP: {}", e))?;
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("Gagal meng-hash password SFTP: {}", e))
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Password acak untuk reset-password; plaintext hanya dikembalikan
/// sekali oleh API, yang disimpan hanya hash-nya.
pub fn generate_password() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
    let mut rng = rand::rngs::OsRng;
    (0..20)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

// ---------------------------------------------------------------------------
// Host key
// ---------------------------------------------------------------------------

fn host_key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("sftp").join("host_key")
}

fn load_or_generate_host_key(data_dir: &Path) -> Result<PrivateKey, String> {
    let path = host_key_path(data_dir);
    if path.exists() {
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| format!("Gagal membaca host key SFTP: {}", e))?;
        return PrivateKey::from_openssh(contents.trim())
            .map_err(|e| format!("Host key SFTP tidak valid: {}", e));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Gagal membuat folder host key SFTP: {}", e))?;
    }
    let key = PrivateKey::random(&mut rand::rngs::OsRng, Algorithm::Ed25519)
        .map_err(|e| format!("Gagal membuat host key SFTP: {}", e))?;
    let encoded = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| format!("Gagal mengenkode host key SFTP: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("Gagal menulis host key SFTP: {}", e))?;
        f.write_all(encoded.as_bytes())
            .map_err(|e| format!("Gagal menulis host key SFTP: {}", e))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, encoded.as_bytes())
            .map_err(|e| format!("Gagal menulis host key SFTP: {}", e))?;
    }
    Ok(key)
}

fn fingerprint_of(key: &PrivateKey) -> String {
    key.public_key().fingerprint(HashAlg::Sha256).to_string()
}

// ---------------------------------------------------------------------------
// Status API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SftpConnectionStatus {
    pub id: String,
    pub username: Option<String>,
    pub remote: String,
    pub connected_at: String,
    pub last_activity: String,
}

#[derive(Debug, Serialize)]
pub struct SftpStatus {
    pub running: bool,
    pub bind_lan: bool,
    pub port: u16,
    pub username: String,
    pub host_key_fingerprint: Option<String>,
    pub root: String,
    pub started_at: Option<String>,
    pub last_error: Option<String>,
    pub connections: Vec<SftpConnectionStatus>,
}

// ---------------------------------------------------------------------------
// State bersama server
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Creds {
    username: String,
    password_hash: Option<String>,
}

struct ConnInfo {
    username: Option<String>,
    remote: String,
    connected_at: DateTime<Utc>,
    last_activity: DateTime<Utc>,
}

struct Shared {
    /// Root workspace yang sudah ter-canonicalize.
    root: PathBuf,
    pool: SqlitePool,
    creds: Arc<std::sync::Mutex<Creds>>,
    connections: Arc<std::sync::Mutex<HashMap<String, ConnInfo>>>,
}

impl Shared {
    fn touch(&self, conn_id: &str) {
        if let Ok(mut conns) = self.connections.lock() {
            if let Some(c) = conns.get_mut(conn_id) {
                c.last_activity = Utc::now();
            }
        }
    }

    async fn log(&self, severity: &str, message: &str) {
        app_log(&self.pool, "sftp", severity, message, None).await;
    }
}

struct Running {
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    session_tasks: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
    started_at: DateTime<Utc>,
    bind_lan: bool,
    port: u16,
}

struct Inner {
    running: Option<Running>,
    last_error: Option<String>,
    fingerprint: Option<String>,
}

/// Handle server SFTP yang disimpan di `AppState`. Server yang sedang
/// berjalan disimpan sebagai `Option<Running>` di dalam handle ini.
pub struct SftpHandle {
    inner: Arc<Mutex<Inner>>,
    creds: Arc<std::sync::Mutex<Creds>>,
    connections: Arc<std::sync::Mutex<HashMap<String, ConnInfo>>>,
}

impl Default for SftpHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl SftpHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                running: None,
                last_error: None,
                fingerprint: None,
            })),
            creds: Arc::new(std::sync::Mutex::new(Creds {
                username: DEFAULT_USERNAME.to_string(),
                password_hash: None,
            })),
            connections: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    fn set_creds(&self, settings: &SftpSettings) {
        if let Ok(mut c) = self.creds.lock() {
            c.username = settings.username.clone();
            c.password_hash = settings.password_hash.clone();
        }
    }

    async fn ensure_fingerprint(&self, data_dir: &Path) -> Option<String> {
        {
            let inner = self.inner.lock().await;
            if let Some(fp) = &inner.fingerprint {
                return Some(fp.clone());
            }
        }
        match load_or_generate_host_key(data_dir) {
            Ok(key) => {
                let fp = fingerprint_of(&key);
                self.inner.lock().await.fingerprint = Some(fp.clone());
                Some(fp)
            }
            Err(e) => {
                self.inner.lock().await.last_error = Some(e);
                None
            }
        }
    }

    pub async fn status(
        &self,
        pool: &SqlitePool,
        workspace_dir: &Path,
        data_dir: &Path,
    ) -> Result<SftpStatus, String> {
        let settings = load_settings(pool).await?;
        let fingerprint = self.ensure_fingerprint(data_dir).await;
        let inner = self.inner.lock().await;
        let (running, bind_lan, port, started_at) = match &inner.running {
            Some(r) => (true, r.bind_lan, r.port, Some(iso_of(r.started_at))),
            None => (false, settings.bind_lan, settings.port, None),
        };
        let last_error = inner.last_error.clone();
        drop(inner);
        let mut connections: Vec<SftpConnectionStatus> = self
            .connections
            .lock()
            .map(|conns| {
                conns
                    .iter()
                    .map(|(id, c)| SftpConnectionStatus {
                        id: id.clone(),
                        username: c.username.clone(),
                        remote: c.remote.clone(),
                        connected_at: iso_of(c.connected_at),
                        last_activity: iso_of(c.last_activity),
                    })
                    .collect()
            })
            .unwrap_or_default();
        connections.sort_by(|a, b| a.connected_at.cmp(&b.connected_at));
        Ok(SftpStatus {
            running,
            bind_lan,
            port,
            username: settings.username,
            host_key_fingerprint: fingerprint,
            root: workspace_dir.to_string_lossy().to_string(),
            started_at,
            last_error,
            connections,
        })
    }

    pub async fn start(
        &self,
        pool: &SqlitePool,
        workspace_dir: &Path,
        data_dir: &Path,
    ) -> Result<SftpStatus, String> {
        let settings = load_settings(pool).await?;
        validate_settings(&settings.username, settings.port)?;
        if settings.password_hash.is_none() {
            return Err(
                "Password SFTP belum diatur. Isi password pada pengaturan SFTP atau gunakan reset password terlebih dahulu."
                    .to_string(),
            );
        }
        self.start_inner(pool, workspace_dir, data_dir, &settings).await?;
        self.status(pool, workspace_dir, data_dir).await
    }

    async fn start_inner(
        &self,
        pool: &SqlitePool,
        workspace_dir: &Path,
        data_dir: &Path,
        settings: &SftpSettings,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().await;
        if inner.running.is_some() {
            return Ok(());
        }
        let root = workspace_dir
            .canonicalize()
            .map_err(|e| format!("Workspace SFTP tidak valid: {}", e))?;
        let key = match load_or_generate_host_key(data_dir) {
            Ok(k) => k,
            Err(e) => {
                inner.last_error = Some(e.clone());
                return Err(e);
            }
        };
        inner.fingerprint = Some(fingerprint_of(&key));
        let bind_addr = if settings.bind_lan { "0.0.0.0" } else { "127.0.0.1" };
        let listener = match TcpListener::bind((bind_addr, settings.port)).await {
            Ok(l) => l,
            Err(e) => {
                let msg = format!(
                    "Gagal membuka port SFTP {}:{}: {}",
                    bind_addr, settings.port, e
                );
                inner.last_error = Some(msg.clone());
                app_log(pool, "sftp", "error", &msg, None).await;
                return Err(msg);
            }
        };
        let actual_port = listener
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(settings.port);
        self.set_creds(settings);
        let config = Arc::new(server::Config {
            keys: vec![key],
            methods: russh::MethodSet::from(&[russh::MethodKind::Password][..]),
            max_auth_attempts: 5,
            nodelay: true,
            ..Default::default()
        });
        let shared = Arc::new(Shared {
            root: root.clone(),
            pool: pool.clone(),
            creds: self.creds.clone(),
            connections: self.connections.clone(),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let session_tasks: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let task = tokio::spawn(accept_loop(
            listener,
            config,
            shared,
            shutdown_rx,
            session_tasks.clone(),
        ));
        inner.running = Some(Running {
            shutdown: shutdown_tx,
            task,
            session_tasks,
            started_at: Utc::now(),
            bind_lan: settings.bind_lan,
            port: actual_port,
        });
        inner.last_error = None;
        drop(inner);
        app_log(
            pool,
            "sftp",
            "info",
            &format!(
                "Server SFTP dimulai di {}:{} (root {})",
                bind_addr,
                actual_port,
                root.display()
            ),
            None,
        )
        .await;
        Ok(())
    }

    async fn stop_inner(&self, pool: &SqlitePool) {
        let running = self.inner.lock().await.running.take();
        if let Some(r) = running {
            let _ = r.shutdown.send(true);
            if let Ok(tasks) = r.session_tasks.lock() {
                for t in tasks.iter() {
                    t.abort();
                }
            }
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), r.task).await;
            if let Ok(mut conns) = self.connections.lock() {
                conns.clear();
            }
            app_log(pool, "sftp", "info", "Server SFTP dihentikan", None).await;
        }
    }

    pub async fn stop(
        &self,
        pool: &SqlitePool,
        workspace_dir: &Path,
        data_dir: &Path,
    ) -> Result<SftpStatus, String> {
        self.stop_inner(pool).await;
        self.status(pool, workspace_dir, data_dir).await
    }

    /// Dipanggil saat core shutdown.
    pub async fn shutdown(&self, pool: &SqlitePool) {
        self.stop_inner(pool).await;
    }

    /// Simpan pengaturan baru; kredensial berlaku langsung, perubahan
    /// port/bind pada server yang berjalan memicu restart.
    pub async fn apply_settings(
        &self,
        pool: &SqlitePool,
        workspace_dir: &Path,
        data_dir: &Path,
        settings: SftpSettings,
    ) -> Result<SftpStatus, String> {
        validate_settings(&settings.username, settings.port)?;
        save_settings(pool, &settings).await?;
        self.set_creds(&settings);
        let restart_needed = {
            let inner = self.inner.lock().await;
            inner
                .running
                .as_ref()
                .map(|r| r.port != settings.port || r.bind_lan != settings.bind_lan)
                .unwrap_or(false)
        };
        if restart_needed {
            self.stop_inner(pool).await;
            self.start_inner(pool, workspace_dir, data_dir, &settings).await?;
        }
        app_log(
            pool,
            "sftp",
            "info",
            &format!(
                "Pengaturan SFTP diperbarui (user {}, port {}, bind_lan {})",
                settings.username, settings.port, settings.bind_lan
            ),
            None,
        )
        .await;
        self.status(pool, workspace_dir, data_dir).await
    }

    /// Buat password acak baru, simpan hash-nya, kembalikan plaintext
    /// (hanya sekali ini; tidak dicatat ke log).
    pub async fn reset_password(
        &self,
        pool: &SqlitePool,
        _workspace_dir: &Path,
        _data_dir: &Path,
    ) -> Result<String, String> {
        let mut settings = load_settings(pool).await?;
        let password = generate_password();
        settings.password_hash = Some(hash_password(&password)?);
        save_settings(pool, &settings).await?;
        self.set_creds(&settings);
        app_log(pool, "sftp", "info", "Password SFTP direset", None).await;
        Ok(password)
    }

    /// Port aktual server yang berjalan (untuk test ephemeral).
    pub async fn running_port(&self) -> Option<u16> {
        self.inner
            .lock()
            .await
            .running
            .as_ref()
            .map(|r| r.port)
    }
}

// ---------------------------------------------------------------------------
// Loop accept & handler SSH
// ---------------------------------------------------------------------------

async fn accept_loop(
    listener: TcpListener,
    config: Arc<server::Config>,
    shared: Arc<Shared>,
    mut shutdown_rx: watch::Receiver<bool>,
    session_tasks: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
) {
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        let shared = shared.clone();
                        let config = config.clone();
                        let handle = tokio::spawn(run_session(stream, peer, config, shared));
                        if let Ok(mut tasks) = session_tasks.lock() {
                            tasks.retain(|t| !t.is_finished());
                            tasks.push(handle.abort_handle());
                        }
                    }
                    Err(e) => {
                        tracing::warn!("SFTP accept gagal: {}", e);
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
}

async fn run_session(
    stream: TcpStream,
    peer: SocketAddr,
    config: Arc<server::Config>,
    shared: Arc<Shared>,
) {
    let conn_id = new_id();
    let remote = peer.to_string();
    if let Ok(mut conns) = shared.connections.lock() {
        let now = Utc::now();
        conns.insert(
            conn_id.clone(),
            ConnInfo {
                username: None,
                remote: remote.clone(),
                connected_at: now,
                last_activity: now,
            },
        );
    }
    shared
        .log("info", &format!("Koneksi SFTP masuk dari {}", remote))
        .await;
    let handler = SshHandler {
        shared: shared.clone(),
        conn_id: conn_id.clone(),
        remote: remote.clone(),
        channels: HashMap::new(),
        authed_user: None,
    };
    match server::run_stream(config, stream, handler).await {
        Ok(session) => {
            let _ = session.await;
        }
        Err(e) => {
            tracing::debug!("Sesi SFTP dari {} berakhir: {}", remote, e);
        }
    }
    if let Ok(mut conns) = shared.connections.lock() {
        conns.remove(&conn_id);
    }
    shared
        .log("info", &format!("Koneksi SFTP dari {} ditutup", remote))
        .await;
}

struct SshHandler {
    shared: Arc<Shared>,
    conn_id: String,
    remote: String,
    channels: HashMap<ChannelId, Channel<Msg>>,
    authed_user: Option<String>,
}

impl server::Handler for SshHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let creds = self
            .shared
            .creds
            .lock()
            .map(|c| c.clone())
            .unwrap_or(Creds {
                username: String::new(),
                password_hash: None,
            });
        // Verifikasi Argon2 dijalankan di thread blocking; tetap
        // diverifikasi walau username salah agar waktunya seragam.
        let password_owned = password.to_string();
        let verified = match creds.password_hash.clone() {
            Some(hash) => tokio::task::spawn_blocking(move || {
                verify_password(&hash, &password_owned)
            })
            .await
            .unwrap_or(false),
            None => false,
        };
        if verified && user == creds.username {
            self.authed_user = Some(user.to_string());
            if let Ok(mut conns) = self.shared.connections.lock() {
                if let Some(c) = conns.get_mut(&self.conn_id) {
                    c.username = Some(user.to_string());
                    c.last_activity = Utc::now();
                }
            }
            self.shared
                .log(
                    "info",
                    &format!("Login SFTP berhasil: {} dari {}", user, self.remote),
                )
                .await;
            Ok(Auth::Accept)
        } else {
            self.shared
                .log(
                    "warn",
                    &format!("Login SFTP gagal: {} dari {}", user, self.remote),
                )
                .await;
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        _public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        _public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        if self.authed_user.is_none() {
            return Ok(false);
        }
        self.channels.insert(channel.id(), channel);
        self.shared.touch(&self.conn_id);
        Ok(true)
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" && self.authed_user.is_some() {
            if let Some(channel) = self.channels.remove(&channel_id) {
                session.channel_success(channel_id)?;
                let sftp = SftpSession::new(self.shared.clone(), self.conn_id.clone());
                russh_sftp::server::run(channel.into_stream(), sftp).await;
                self.shared.touch(&self.conn_id);
                return Ok(());
            }
        }
        session.channel_failure(channel_id)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Resolusi path virtual -> filesystem (containment workspace)
// ---------------------------------------------------------------------------

/// Normalisasi path virtual SFTP menjadi komponen di bawah root.
/// `..` yang keluar root ditolak.
fn normalize_virtual(path: &str) -> Result<Vec<String>, StatusCode> {
    let mut parts: Vec<String> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(StatusCode::PermissionDenied);
                }
            }
            _ => parts.push(part.to_string()),
        }
    }
    Ok(parts)
}

fn parts_to_virtual(parts: &[String]) -> String {
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn map_io_err(e: &std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

/// Path harus ada; symlink diikuti dan hasil canonicalize harus tetap
/// di dalam root.
fn resolve_existing(root: &Path, vpath: &str) -> Result<PathBuf, StatusCode> {
    let parts = normalize_virtual(vpath)?;
    let joined = root.join(parts.join("/"));
    let canon = joined.canonicalize().map_err(|e| map_io_err(&e))?;
    if !canon.starts_with(root) {
        return Err(StatusCode::PermissionDenied);
    }
    Ok(canon)
}

/// Path boleh belum ada (untuk membuat file/folder baru). Ancestor
/// terdekat yang ada di-canonicalize dan harus di dalam root; symlink
/// menggantung (target tidak ada) ditolak agar tidak menulis keluar.
fn resolve_create(root: &Path, vpath: &str) -> Result<PathBuf, StatusCode> {
    let parts = normalize_virtual(vpath)?;
    if parts.is_empty() {
        return Ok(root.to_path_buf());
    }
    let joined = root.join(parts.join("/"));
    if let Ok(md) = joined.symlink_metadata() {
        if md.file_type().is_symlink() {
            // Symlink yang ada: target harus ter-canonicalize di root.
            let canon = joined
                .canonicalize()
                .map_err(|_| StatusCode::PermissionDenied)?;
            if !canon.starts_with(root) {
                return Err(StatusCode::PermissionDenied);
            }
            return Ok(joined);
        }
        let canon = joined.canonicalize().map_err(|e| map_io_err(&e))?;
        if !canon.starts_with(root) {
            return Err(StatusCode::PermissionDenied);
        }
        return Ok(canon);
    }
    // Belum ada: canonicalize ancestor terdekat yang ada.
    let mut ancestor = joined.clone();
    let mut rest: Vec<String> = Vec::new();
    loop {
        if ancestor.symlink_metadata().is_ok() {
            break;
        }
        match ancestor.file_name() {
            Some(name) => {
                rest.push(name.to_string_lossy().to_string());
                ancestor.pop();
            }
            None => return Err(StatusCode::Failure),
        }
    }
    let mut resolved = ancestor
        .canonicalize()
        .map_err(|_| StatusCode::PermissionDenied)?;
    if !resolved.starts_with(root) {
        return Err(StatusCode::PermissionDenied);
    }
    for part in rest.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

/// Seperti `resolve_create` tetapi komponen terakhir tidak diikuti bila
/// symlink (untuk lstat/readlink/remove/rename pada link itu sendiri).
/// Parent tetap di-canonicalize dan harus di dalam root.
fn resolve_link(root: &Path, vpath: &str) -> Result<PathBuf, StatusCode> {
    let parts = normalize_virtual(vpath)?;
    if parts.is_empty() {
        return Ok(root.to_path_buf());
    }
    let (last, parents) = parts.split_last().unwrap();
    let parent = if parents.is_empty() {
        root.to_path_buf()
    } else {
        resolve_existing(root, &parts_to_virtual(parents))?
    };
    Ok(parent.join(last))
}

/// Bila `path` adalah symlink dengan target yang ada, target harus
/// tetap di dalam root. Symlink menggantung tidak ditolak di sini;
/// pemanggil memutuskan (mis. remove link menggantung diperbolehkan).
fn check_symlink_target_inside(root: &Path, path: &Path) -> Result<(), StatusCode> {
    if let Ok(md) = path.symlink_metadata() {
        if md.file_type().is_symlink() {
            if let Ok(canon) = path.canonicalize() {
                if !canon.starts_with(root) {
                    return Err(StatusCode::PermissionDenied);
                }
            }
        }
    }
    Ok(())
}

fn attrs_of(md: &std::fs::Metadata) -> FileAttributes {
    let mut attrs = FileAttributes::from(md);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        attrs.permissions = Some(md.mode());
    }
    attrs
}

fn ok_status(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en-US".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Handler SFTP
// ---------------------------------------------------------------------------

enum OpenHandle {
    File { file: std::fs::File },
    Dir {
        entries: Vec<File>,
        offset: usize,
    },
}

struct SftpSession {
    shared: Arc<Shared>,
    conn_id: String,
    handles: HashMap<String, OpenHandle>,
    next_handle: u64,
    version: Option<u32>,
}

impl SftpSession {
    fn new(shared: Arc<Shared>, conn_id: String) -> Self {
        Self {
            shared,
            conn_id,
            handles: HashMap::new(),
            next_handle: 0,
            version: None,
        }
    }

    fn root(&self) -> &Path {
        &self.shared.root
    }

    fn new_handle(&mut self, handle: OpenHandle) -> String {
        self.next_handle += 1;
        let id = format!("h{}", self.next_handle);
        self.handles.insert(id.clone(), handle);
        id
    }

    fn file_handle(&mut self, handle: &str) -> Result<&mut std::fs::File, StatusCode> {
        match self.handles.get_mut(handle) {
            Some(OpenHandle::File { file }) => Ok(file),
            _ => Err(StatusCode::Failure),
        }
    }
}

impl russh_sftp::server::Handler for SftpSession {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<russh_sftp::protocol::Version, Self::Error> {
        if self.version.is_some() {
            return Err(StatusCode::ConnectionLost);
        }
        self.version = Some(version);
        self.shared.touch(&self.conn_id);
        Ok(russh_sftp::protocol::Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        self.shared.touch(&self.conn_id);
        if !pflags.contains(OpenFlags::READ)
            && !pflags.contains(OpenFlags::WRITE)
            && !pflags.contains(OpenFlags::APPEND)
        {
            return Err(StatusCode::Failure);
        }
        let path = if pflags.contains(OpenFlags::CREATE) {
            resolve_create(self.root(), &filename)?
        } else {
            resolve_existing(self.root(), &filename)?
        };
        if let Ok(md) = std::fs::metadata(&path) {
            if md.is_dir() {
                return Err(StatusCode::Failure);
            }
        }
        let options = std::fs::OpenOptions::from(pflags);
        let file = options.open(&path).map_err(|e| map_io_err(&e))?;
        let handle = self.new_handle(OpenHandle::File { file });
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        match self.handles.remove(&handle) {
            Some(_) => Ok(ok_status(id)),
            None => Err(StatusCode::Failure),
        }
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        self.shared.touch(&self.conn_id);
        let file = self.file_handle(&handle)?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| map_io_err(&e))?;
        let want = len.min(MAX_READ_LEN) as usize;
        let mut buf = vec![0u8; want];
        let mut read_total = 0usize;
        while read_total < want {
            match file.read(&mut buf[read_total..]) {
                Ok(0) => break,
                Ok(n) => read_total += n,
                Err(e) => return Err(map_io_err(&e)),
            }
        }
        if read_total == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(read_total);
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        let file = self.file_handle(&handle)?;
        // Dengan O_APPEND seek diabaikan OS untuk penulisan, sesuai
        // semantik APPEND SFTP; tanpa APPEND offset dihormati.
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| map_io_err(&e))?;
        file.write_all(&data).map_err(|e| map_io_err(&e))?;
        Ok(ok_status(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.shared.touch(&self.conn_id);
        let resolved = resolve_link(self.root(), &path)?;
        let md = resolved.symlink_metadata().map_err(|e| map_io_err(&e))?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&md),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.shared.touch(&self.conn_id);
        let resolved = resolve_existing(self.root(), &path)?;
        let md = std::fs::metadata(&resolved).map_err(|e| map_io_err(&e))?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&md),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        self.shared.touch(&self.conn_id);
        let file = self.file_handle(&handle)?;
        let md = file.metadata().map_err(|e| map_io_err(&e))?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&md),
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        // Minimal sesuai kontrak: hanya truncate bila size diberikan.
        if let Some(size) = attrs.size {
            let resolved = resolve_existing(self.root(), &path)?;
            let md = std::fs::metadata(&resolved).map_err(|e| map_io_err(&e))?;
            if !md.is_file() {
                return Err(StatusCode::Failure);
            }
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&resolved)
                .map_err(|e| map_io_err(&e))?;
            file.set_len(size).map_err(|e| map_io_err(&e))?;
        }
        Ok(ok_status(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        if let Some(size) = attrs.size {
            let file = self.file_handle(&handle)?;
            file.set_len(size).map_err(|e| map_io_err(&e))?;
        }
        Ok(ok_status(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        self.shared.touch(&self.conn_id);
        let resolved = resolve_existing(self.root(), &path)?;
        let md = std::fs::metadata(&resolved).map_err(|e| map_io_err(&e))?;
        if !md.is_dir() {
            return Err(StatusCode::Failure);
        }
        let mut entries: Vec<File> = Vec::new();
        let read_dir = std::fs::read_dir(&resolved).map_err(|e| map_io_err(&e))?;
        for entry in read_dir {
            let entry = entry.map_err(|e| map_io_err(&e))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let attrs = entry
                .metadata()
                .map(|m| attrs_of(&m))
                .unwrap_or_default();
            entries.push(File::new(name, attrs));
        }
        entries.sort_by(|a, b| a.filename.cmp(&b.filename));
        let handle = self.new_handle(OpenHandle::Dir {
            entries,
            offset: 0,
        });
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        self.shared.touch(&self.conn_id);
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Dir { entries, offset }) => {
                if *offset >= entries.len() {
                    return Err(StatusCode::Eof);
                }
                let end = (*offset + READDIR_BATCH).min(entries.len());
                let files: Vec<File> = entries[*offset..end].to_vec();
                *offset = end;
                Ok(Name { id, files })
            }
            _ => Err(StatusCode::Failure),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        let parts = normalize_virtual(&filename)?;
        if parts.is_empty() {
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = resolve_link(self.root(), &filename)?;
        let md = resolved.symlink_metadata().map_err(|e| map_io_err(&e))?;
        if md.is_dir() {
            return Err(StatusCode::Failure);
        }
        check_symlink_target_inside(self.root(), &resolved)?;
        std::fs::remove_file(&resolved).map_err(|e| map_io_err(&e))?;
        Ok(ok_status(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        let parts = normalize_virtual(&path)?;
        if parts.is_empty() {
            return Err(StatusCode::Failure);
        }
        let resolved = resolve_create(self.root(), &path)?;
        if resolved.symlink_metadata().is_ok() {
            return Err(StatusCode::Failure);
        }
        std::fs::create_dir(&resolved).map_err(|e| map_io_err(&e))?;
        Ok(ok_status(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        let parts = normalize_virtual(&path)?;
        if parts.is_empty() {
            return Err(StatusCode::PermissionDenied);
        }
        let resolved = resolve_link(self.root(), &path)?;
        let md = resolved.symlink_metadata().map_err(|e| map_io_err(&e))?;
        if md.file_type().is_symlink() || !md.is_dir() {
            return Err(StatusCode::Failure);
        }
        std::fs::remove_dir(&resolved).map_err(|e| map_io_err(&e))?;
        Ok(ok_status(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        self.shared.touch(&self.conn_id);
        let src_parts = normalize_virtual(&oldpath)?;
        let dst_parts = normalize_virtual(&newpath)?;
        if src_parts.is_empty() || dst_parts.is_empty() {
            return Err(StatusCode::PermissionDenied);
        }
        let src = resolve_link(self.root(), &oldpath)?;
        let src_md = src.symlink_metadata().map_err(|e| map_io_err(&e))?;
        check_symlink_target_inside(self.root(), &src)?;
        let dst = resolve_link(self.root(), &newpath)?;
        // Folder tidak boleh dipindah ke dalam dirinya sendiri.
        if src_md.is_dir() {
            let src_canon = resolve_existing(self.root(), &oldpath)?;
            if dst.starts_with(&src_canon) && dst != src {
                return Err(StatusCode::Failure);
            }
        }
        if let Ok(dst_md) = dst.symlink_metadata() {
            if dst_md.is_dir() {
                if !src_md.is_dir() {
                    return Err(StatusCode::Failure);
                }
                std::fs::remove_dir(&dst).map_err(|e| map_io_err(&e))?;
            } else {
                if src_md.is_dir() {
                    return Err(StatusCode::Failure);
                }
                check_symlink_target_inside(self.root(), &dst)?;
                std::fs::remove_file(&dst).map_err(|e| map_io_err(&e))?;
            }
        }
        std::fs::rename(&src, &dst).map_err(|e| map_io_err(&e))?;
        Ok(ok_status(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        self.shared.touch(&self.conn_id);
        let parts = normalize_virtual(&path)?;
        let virtual_path = match resolve_existing(self.root(), &path) {
            Ok(canon) => {
                let rel = canon
                    .strip_prefix(self.root())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if rel.is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", rel.replace('\\', "/"))
                }
            }
            Err(_) => parts_to_virtual(&parts),
        };
        Ok(Name {
            id,
            files: vec![File::dummy(virtual_path)],
        })
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        self.shared.touch(&self.conn_id);
        let resolved = resolve_link(self.root(), &path)?;
        let target = std::fs::read_link(&resolved).map_err(|e| map_io_err(&e))?;
        // Target absolut harus di dalam root; target relatif
        // dinormalisasi terhadap folder virtual link.
        let virtual_target = if target.is_absolute() {
            let target_str = target.to_string_lossy().to_string();
            let root_str = self.root().to_string_lossy().to_string();
            match target_str.strip_prefix(&root_str) {
                Some(rest) => {
                    let rest = rest.trim_start_matches('/');
                    if rest.is_empty() {
                        "/".to_string()
                    } else {
                        format!("/{}", rest)
                    }
                }
                None => return Err(StatusCode::PermissionDenied),
            }
        } else {
            let parts = normalize_virtual(&path)?;
            let mut base: Vec<String> = parts[..parts.len().saturating_sub(1)].to_vec();
            for part in target.to_string_lossy().split('/') {
                match part {
                    "" | "." => {}
                    ".." => {
                        if base.pop().is_none() {
                            return Err(StatusCode::PermissionDenied);
                        }
                    }
                    _ => base.push(part.to_string()),
                }
            }
            parts_to_virtual(&base)
        };
        Ok(Name {
            id,
            files: vec![File::dummy(virtual_target)],
        })
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wf-sftp-test-{}-{}", tag, new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn normalize_rejects_escape() {
        assert_eq!(
            normalize_virtual("/../etc/passwd"),
            Err(StatusCode::PermissionDenied)
        );
        assert_eq!(
            normalize_virtual("/a/../../b"),
            Err(StatusCode::PermissionDenied)
        );
        assert_eq!(
            normalize_virtual("/a/../b").unwrap(),
            vec!["b".to_string()]
        );
        assert_eq!(normalize_virtual("/").unwrap(), Vec::<String>::new());
        assert_eq!(
            normalize_virtual("relatif/file.txt").unwrap(),
            vec!["relatif".to_string(), "file.txt".to_string()]
        );
    }

    #[test]
    fn resolve_rejects_traversal_and_symlink_escape() {
        let root = temp_dir("root");
        let outside = temp_dir("outside");
        std::fs::write(outside.join("rahasia.txt"), b"rahasia").unwrap();
        std::fs::write(root.join("ok.txt"), b"ok").unwrap();

        // File biasa di dalam root lolos.
        assert!(resolve_existing(&root, "/ok.txt").is_ok());
        // Traversal ditolak.
        assert_eq!(
            resolve_existing(&root, "/../rahasia.txt"),
            Err(StatusCode::PermissionDenied)
        );
        // Path baru di dalam root boleh.
        let created = resolve_create(&root, "/baru.txt").unwrap();
        assert!(created.starts_with(&root));

        #[cfg(unix)]
        {
            // Symlink ke file di luar root ditolak untuk baca/tulis.
            std::os::unix::fs::symlink(outside.join("rahasia.txt"), root.join("link-keluar"))
                .unwrap();
            assert_eq!(
                resolve_existing(&root, "/link-keluar"),
                Err(StatusCode::PermissionDenied)
            );
            assert_eq!(
                resolve_create(&root, "/link-keluar"),
                Err(StatusCode::PermissionDenied)
            );
            assert_eq!(
                check_symlink_target_inside(&root, &root.join("link-keluar")),
                Err(StatusCode::PermissionDenied)
            );
            // Symlink folder keluar: membuat file lewat link ditolak.
            std::os::unix::fs::symlink(&outside, root.join("dir-keluar")).unwrap();
            assert_eq!(
                resolve_create(&root, "/dir-keluar/baru.txt"),
                Err(StatusCode::PermissionDenied)
            );
            assert_eq!(
                resolve_existing(&root, "/dir-keluar/rahasia.txt"),
                Err(StatusCode::PermissionDenied)
            );
            // Symlink ke dalam root tetap boleh.
            std::os::unix::fs::symlink(root.join("ok.txt"), root.join("link-dalam")).unwrap();
            assert!(resolve_existing(&root, "/link-dalam").is_ok());
        }

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn password_hash_roundtrip() {
        let hash = hash_password("kata-sandi-uji").unwrap();
        assert!(verify_password(&hash, "kata-sandi-uji"));
        assert!(!verify_password(&hash, "salah"));
        assert!(!hash.contains("kata-sandi-uji"));
    }

    struct TestClient;

    impl russh::client::Handler for TestClient {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &russh::keys::ssh_key::PublicKey,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn integration_auth_rejected_and_transfer_works() {
        let root = temp_dir("integ-root");
        let data_dir = temp_dir("integ-data");
        // Pool in-memory tanpa tabel: app_log gagal diam-diam (diabaikan),
        // start_inner tidak membaca sftp_settings.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let settings = SftpSettings {
            username: "penguji".to_string(),
            password_hash: Some(hash_password("sandi-benar").unwrap()),
            port: 0, // ephemeral untuk test
            bind_lan: false,
        };
        let handle = SftpHandle::new();
        handle
            .start_inner(&pool, &root, &data_dir, &settings)
            .await
            .unwrap();
        let port = handle.running_port().await.expect("server harus berjalan");

        let client_config = StdArc::new(russh::client::Config::default());

        // Auth dengan password salah ditolak.
        let mut bad = russh::client::connect(client_config.clone(), ("127.0.0.1", port), TestClient)
            .await
            .unwrap();
        let bad_auth = bad
            .authenticate_password("penguji", "sandi-salah")
            .await
            .unwrap();
        assert!(!bad_auth.success(), "password salah harus ditolak");
        drop(bad);

        // Auth benar: list, upload, download file kecil.
        let mut session =
            russh::client::connect(client_config, ("127.0.0.1", port), TestClient)
                .await
                .unwrap();
        let auth = session
            .authenticate_password("penguji", "sandi-benar")
            .await
            .unwrap();
        assert!(auth.success(), "password benar harus diterima");
        let channel = session.channel_open_session().await.unwrap();
        channel.request_subsystem(true, "sftp").await.unwrap();
        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .unwrap();

        let payload = b"isi file sftp webforge";
        // Client write() russh-sftp membuka tanpa CREATE; pakai create().
        {
            use tokio::io::AsyncWriteExt;
            let mut file = sftp.create("/halo.txt").await.unwrap();
            file.write_all(payload).await.unwrap();
            file.shutdown().await.unwrap();
        }

        let names: Vec<String> = sftp
            .read_dir("/")
            .await
            .unwrap()
            .map(|entry| entry.file_name())
            .collect();
        assert!(
            names.iter().any(|n| n == "halo.txt"),
            "readdir harus memuat halo.txt, dapat: {:?}",
            names
        );

        let downloaded = sftp.read("/halo.txt").await.unwrap();
        assert_eq!(downloaded, payload, "file hasil download harus sama");

        // File benar-benar tertulis di workspace.
        let on_disk = std::fs::read(root.join("halo.txt")).unwrap();
        assert_eq!(on_disk, payload);

        // Traversal lewat protokol ditolak.
        assert!(sftp.read("/../rahasia.txt").await.is_err());

        sftp.close().await.ok();
        handle.shutdown(&pool).await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&data_dir).ok();
    }
}
