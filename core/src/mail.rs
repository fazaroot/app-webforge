//! Pengaturan SMTP dan pengiriman mail uji WebForge Mobile v0.5.0.
//!
//! Password SMTP disimpan terenkripsi (secrets.rs) di user_settings dan
//! tidak pernah dikembalikan ke frontend (hanya `has_password`).
//! Konfigurasi terdekripsi ditulis ke `data/mail/smtp.json` (0600)
//! yang dibaca shim `webforge-sendmail` (libsendmail.so) saat PHP
//! memanggil mail(). Tanpa konfigurasi, smtp.json tidak ada dan shim
//! gagal dengan pesan jujur.
//!
//! Modul ini didaftarkan dari routes/mod.rs lewat #[path] agar tetap
//! berada di core/src/mail.rs sesuai kontrak worker B.

use crate::state::AppState;
use crate::util::{db_err, ApiError, ApiResult};
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    None,
    Starttls,
    Tls,
}

impl SmtpSecurity {
    pub fn as_str(&self) -> &'static str {
        match self {
            SmtpSecurity::None => "none",
            SmtpSecurity::Starttls => "starttls",
            SmtpSecurity::Tls => "tls",
        }
    }
    fn from_str_loose(s: &str) -> SmtpSecurity {
        match s.trim().to_ascii_lowercase().as_str() {
            "starttls" => SmtpSecurity::Starttls,
            "tls" => SmtpSecurity::Tls,
            _ => SmtpSecurity::None,
        }
    }
}

/// Konfigurasi SMTP terdekripsi (password plaintext hanya hidup di
/// memori dan di smtp.json 0600 untuk shim).
#[derive(Debug, Clone, PartialEq)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

impl SmtpConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("Host SMTP wajib diisi".to_string());
        }
        if self.host.len() > 253
            || !self
                .host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            return Err("Host SMTP tidak valid".to_string());
        }
        if self.port == 0 {
            return Err("Port SMTP harus antara 1 dan 65535".to_string());
        }
        let from = self.from.trim();
        if from.is_empty() || !from.contains('@') || from.contains(char::is_whitespace) {
            return Err("Alamat pengirim (from) tidak valid".to_string());
        }
        if let Some(user) = &self.username {
            if user.len() > 320 {
                return Err("Username SMTP terlalu panjang".to_string());
            }
        }
        Ok(())
    }

    fn to_json(&self) -> Value {
        json!({
            "host": self.host,
            "port": self.port,
            "security": self.security.as_str(),
            "username": self.username,
            "password": self.password,
            "from": self.from,
        })
    }
}

pub fn smtp_json_path(data_dir: &Path) -> PathBuf {
    data_dir.join("mail").join("smtp.json")
}

/// Tulis smtp.json (0600) dari konfigurasi terdekripsi, atau hapus
/// berkasnya bila SMTP tidak terkonfigurasi.
pub fn write_smtp_json(data_dir: &Path, smtp: Option<&SmtpConfig>) -> std::io::Result<PathBuf> {
    let path = smtp_json_path(data_dir);
    match smtp {
        Some(cfg) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let text = serde_json::to_string_pretty(&cfg.to_json())
                .unwrap_or_else(|_| "{}".to_string());
            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&path)?;
                f.write_all(text.as_bytes())?;
            }
            #[cfg(not(unix))]
            {
                std::fs::write(&path, text)?;
            }
            Ok(path)
        }
        None => {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            Ok(path)
        }
    }
}

async fn get_setting(pool: &sqlx::SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM user_settings WHERE key=?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

fn parse_setting_string(raw: &str) -> String {
    serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_string())
}

async fn set_setting(pool: &sqlx::SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_settings (key, value, updated_at) VALUES (?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(crate::util::now_iso())
    .execute(pool)
    .await?;
    Ok(())
}

async fn delete_setting(pool: &sqlx::SqlitePool, key: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM user_settings WHERE key=?1")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// Muat konfigurasi SMTP terdekripsi. None bila host belum diatur.
pub async fn load_smtp_config(pool: &sqlx::SqlitePool, data_dir: &Path) -> Option<SmtpConfig> {
    let host = get_setting(pool, "mail.host").await.map(|v| parse_setting_string(&v))?;
    if host.trim().is_empty() {
        return None;
    }
    let port = get_setting(pool, "mail.port")
        .await
        .and_then(|v| v.trim().parse::<u16>().ok())
        .unwrap_or(587);
    let security = get_setting(pool, "mail.security")
        .await
        .map(|v| SmtpSecurity::from_str_loose(&parse_setting_string(&v)))
        .unwrap_or(SmtpSecurity::Starttls);
    let username = get_setting(pool, "mail.username")
        .await
        .map(|v| parse_setting_string(&v))
        .filter(|s| !s.trim().is_empty());
    let from = get_setting(pool, "mail.from")
        .await
        .map(|v| parse_setting_string(&v))
        .unwrap_or_default();
    let password = match get_setting(pool, "mail.password_encrypted").await {
        Some(enc) => {
            let enc = parse_setting_string(&enc);
            if enc.trim().is_empty() {
                None
            } else {
                crate::secrets::decrypt_secret(data_dir, &enc).ok()
            }
        }
        None => None,
    };
    Some(SmtpConfig {
        host,
        port,
        security,
        username,
        password,
        from,
    })
}

async fn has_password(pool: &sqlx::SqlitePool) -> bool {
    get_setting(pool, "mail.password_encrypted")
        .await
        .map(|v| !parse_setting_string(&v).trim().is_empty())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize)]
pub struct MailSettingsOut {
    pub configured: bool,
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: Option<String>,
    pub from: String,
    pub has_password: bool,
}

async fn settings_out(state: &AppState) -> MailSettingsOut {
    let cfg = load_smtp_config(&state.pool, &state.config.data_dir).await;
    match cfg {
        Some(cfg) => MailSettingsOut {
            configured: true,
            host: cfg.host,
            port: cfg.port,
            security: cfg.security,
            username: cfg.username,
            from: cfg.from,
            has_password: has_password(&state.pool).await,
        },
        None => MailSettingsOut {
            configured: false,
            host: String::new(),
            port: 587,
            security: SmtpSecurity::Starttls,
            username: None,
            from: String::new(),
            has_password: has_password(&state.pool).await,
        },
    }
}

pub async fn get_mail_settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(settings_out(&state).await).unwrap_or(json!({}))))
}

#[derive(Debug, Deserialize)]
pub struct PutMailSettings {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: Option<String>,
    pub from: String,
    /// Password baru (write-only). None = pertahankan password lama.
    pub password: Option<String>,
    /// true = hapus password tersimpan.
    pub clear_password: Option<bool>,
}

pub async fn put_mail_settings(
    State(state): State<AppState>,
    Json(body): Json<PutMailSettings>,
) -> ApiResult<Json<Value>> {
    let host = body.host.trim().to_string();
    if host.is_empty() {
        // Host kosong = matikan konfigurasi SMTP.
        for key in ["mail.host", "mail.port", "mail.security", "mail.username", "mail.from", "mail.password_encrypted"] {
            delete_setting(&state.pool, key).await.map_err(db_err)?;
        }
        write_smtp_json(&state.config.data_dir, None)
            .map_err(|e| ApiError::internal(format!("Gagal menghapus smtp.json: {}", e)))?;
        return Ok(Json(serde_json::to_value(settings_out(&state).await).unwrap_or(json!({}))));
    }

    let username = body
        .username
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let cfg = SmtpConfig {
        host,
        port: body.port,
        security: body.security,
        username,
        password: None,
        from: body.from.trim().to_string(),
    };
    cfg.validate().map_err(ApiError::bad_request)?;

    set_setting(&state.pool, "mail.host", &serde_json::to_string(&cfg.host).unwrap())
        .await
        .map_err(db_err)?;
    set_setting(&state.pool, "mail.port", &cfg.port.to_string())
        .await
        .map_err(db_err)?;
    set_setting(
        &state.pool,
        "mail.security",
        &serde_json::to_string(cfg.security.as_str()).unwrap(),
    )
    .await
    .map_err(db_err)?;
    match &cfg.username {
        Some(u) => set_setting(&state.pool, "mail.username", &serde_json::to_string(u).unwrap())
            .await
            .map_err(db_err)?,
        None => delete_setting(&state.pool, "mail.username").await.map_err(db_err)?,
    }
    set_setting(&state.pool, "mail.from", &serde_json::to_string(&cfg.from).unwrap())
        .await
        .map_err(db_err)?;

    if body.clear_password.unwrap_or(false) {
        delete_setting(&state.pool, "mail.password_encrypted")
            .await
            .map_err(db_err)?;
    } else if let Some(password) = body.password.filter(|p| !p.is_empty()) {
        let encrypted = crate::secrets::encrypt_secret(&state.config.data_dir, &password)
            .map_err(ApiError::internal)?;
        set_setting(
            &state.pool,
            "mail.password_encrypted",
            &serde_json::to_string(&encrypted).unwrap(),
        )
        .await
        .map_err(db_err)?;
    }

    // Sinkronkan smtp.json + php.ini dari konfigurasi terdekripsi.
    let loaded = load_smtp_config(&state.pool, &state.config.data_dir).await;
    let settings = crate::php::load_php_settings(&state.pool).await;
    crate::php::write_php_ini_with_smtp(&state.config.data_dir, &settings, loaded.as_ref())
        .map_err(|e| ApiError::internal(format!("Gagal menulis smtp.json/php.ini: {}", e)))?;

    Ok(Json(serde_json::to_value(settings_out(&state).await).unwrap_or(json!({}))))
}

#[derive(Debug, Deserialize)]
pub struct TestMailBody {
    pub to: String,
}

const MAIL_TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Kirim email uji dengan menjalankan shim sendmail sungguhan.
pub async fn test_mail(
    State(state): State<AppState>,
    Json(body): Json<TestMailBody>,
) -> ApiResult<Json<Value>> {
    let started = Instant::now();
    let to = body.to.trim().to_string();
    if to.is_empty() || !to.contains('@') || to.contains(char::is_whitespace) {
        return Err(ApiError::bad_request("Alamat tujuan (to) tidak valid"));
    }
    let cfg = load_smtp_config(&state.pool, &state.config.data_dir).await;
    let Some(cfg) = cfg else {
        return Ok(Json(json!({
            "ok": false,
            "error": "SMTP belum dikonfigurasi: isi host/port/from di pengaturan mail terlebih dahulu",
            "output": "",
            "duration_ms": started.elapsed().as_millis() as u64,
        })));
    };
    // Pastikan smtp.json mutakhir sebelum shim dijalankan.
    if let Err(e) = write_smtp_json(&state.config.data_dir, Some(&cfg)) {
        return Ok(Json(json!({
            "ok": false,
            "error": format!("Gagal menulis smtp.json: {}", e),
            "output": "",
            "duration_ms": started.elapsed().as_millis() as u64,
        })));
    }
    let Some(shim) = crate::php::find_sendmail_binary(state.native_lib_dir.as_deref()) else {
        return Ok(Json(json!({
            "ok": false,
            "error": "Binary shim sendmail tidak ditemukan: libsendmail.so tidak ada di nativeLibraryDir dan webforge-sendmail tidak ada di PATH",
            "output": "",
            "duration_ms": started.elapsed().as_millis() as u64,
        })));
    };
    let message = format!(
        "From: {}\r\nTo: {}\r\nSubject: WebForge Mobile — email uji\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nIni email uji dari WebForge Mobile. Konfigurasi SMTP berfungsi.\r\n",
        cfg.from, to
    );
    let config_path = smtp_json_path(&state.config.data_dir);
    let mut cmd = tokio::process::Command::new(&shim);
    cmd.arg("--config")
        .arg(&config_path)
        .arg("-t")
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    crate::util::prepare_spawn(
        &mut cmd,
        state.native_lib_dir.as_deref(),
        &state.config.data_dir,
        Some(&state.config.data_dir),
    );
    let run = async {
        let mut child = cmd.spawn().map_err(|e| format!("Gagal menjalankan shim sendmail: {}", e))?;
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(message.as_bytes())
                .await
                .map_err(|e| format!("Gagal menulis pesan ke shim: {}", e))?;
        }
        child
            .wait_with_output()
            .await
            .map_err(|e| format!("Shim sendmail gagal: {}", e))
    };
    let duration = |started: Instant| started.elapsed().as_millis() as u64;
    match tokio::time::timeout(MAIL_TEST_TIMEOUT, run).await {
        Ok(Ok(out)) => {
            let output = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
            .trim()
            .to_string();
            if out.status.success() {
                Ok(Json(json!({
                    "ok": true,
                    "error": null,
                    "output": output,
                    "duration_ms": duration(started),
                })))
            } else {
                Ok(Json(json!({
                    "ok": false,
                    "error": if output.is_empty() {
                        format!("Shim sendmail keluar dengan kode {}", out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".to_string()))
                    } else {
                        output.clone()
                    },
                    "output": output,
                    "duration_ms": duration(started),
                })))
            }
        }
        Ok(Err(e)) => Ok(Json(json!({
            "ok": false,
            "error": e,
            "output": "",
            "duration_ms": duration(started),
        }))),
        Err(_) => Ok(Json(json!({
            "ok": false,
            "error": format!("Pengiriman email uji timeout setelah {} detik", MAIL_TEST_TIMEOUT.as_secs()),
            "output": "",
            "duration_ms": duration(started),
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> SmtpConfig {
        SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            security: SmtpSecurity::Starttls,
            username: Some("user@example.com".to_string()),
            password: Some("rahasia".to_string()),
            from: "WebForge <no-reply@example.com>".to_string(),
        }
    }

    #[test]
    fn smtp_config_validation() {
        let mut cfg = sample_config();
        cfg.from = "no-reply@example.com".to_string();
        assert!(cfg.validate().is_ok());
        cfg.host = "smtp example.com".to_string();
        assert!(cfg.validate().is_err());
        cfg.host = "smtp.example.com".to_string();
        cfg.port = 0;
        assert!(cfg.validate().is_err());
        cfg.port = 587;
        cfg.from = "bukan-email".to_string();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn smtp_json_written_and_removed() {
        let dir = std::env::temp_dir().join(format!("wf-mail-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = sample_config();
        let path = write_smtp_json(&dir, Some(&cfg)).unwrap();
        assert!(path.is_file());
        let text = std::fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["host"], "smtp.example.com");
        assert_eq!(v["port"], 587);
        assert_eq!(v["security"], "starttls");
        assert_eq!(v["password"], "rahasia");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        write_smtp_json(&dir, None).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn smtp_settings_roundtrip_encrypted() {
        let dir = std::env::temp_dir().join(format!("wf-mail-db-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE user_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();

        assert!(load_smtp_config(&pool, &dir).await.is_none());
        assert!(!has_password(&pool).await);

        set_setting(&pool, "mail.host", "\"smtp.example.com\"").await.unwrap();
        set_setting(&pool, "mail.port", "465").await.unwrap();
        set_setting(&pool, "mail.security", "\"tls\"").await.unwrap();
        set_setting(&pool, "mail.from", "\"no-reply@example.com\"").await.unwrap();
        let enc = crate::secrets::encrypt_secret(&dir, "kata-sandi").unwrap();
        set_setting(&pool, "mail.password_encrypted", &serde_json::to_string(&enc).unwrap())
            .await
            .unwrap();

        let cfg = load_smtp_config(&pool, &dir).await.unwrap();
        assert_eq!(cfg.host, "smtp.example.com");
        assert_eq!(cfg.port, 465);
        assert_eq!(cfg.security, SmtpSecurity::Tls);
        assert_eq!(cfg.password.as_deref(), Some("kata-sandi"));
        assert!(has_password(&pool).await);
        // Ciphertext tersimpan tidak memuat plaintext.
        let raw = get_setting(&pool, "mail.password_encrypted").await.unwrap();
        assert!(!raw.contains("kata-sandi"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
