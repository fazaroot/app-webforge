//! PHP runtime: deteksi binary nyata, pengaturan php.ini, probe info,
//! dan logika restart server.
//!
//! Binary PHP di Android adalah `libphp.so` (PHP CLI hasil cross-compile,
//! dikemas sebagai .so agar sah dieksekusi dari nativeLibraryDir — W^X).
//! Di host/test, binary `php` dari PATH dipakai bila ada. Tidak ada daftar
//! ekstensi atau versi yang dikarang: semua berasal dari menjalankan
//! `php -v` dan `php -m` sungguhan.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MAX_RESTART_ATTEMPTS: u32 = 3;

/// Jeda sebelum restart otomatis ke-(completed_attempts+1).
/// `completed_attempts` = jumlah restart yang sudah dilakukan.
/// Mengembalikan None bila anggaran restart (maks 3) habis.
pub fn restart_delay(completed_attempts: u32) -> Option<Duration> {
    match completed_attempts {
        0 => Some(Duration::from_secs(2)),
        1 => Some(Duration::from_secs(5)),
        2 => Some(Duration::from_secs(10)),
        _ => None,
    }
}

/// Cari binary PHP nyata: `libphp.so` di nativeLibraryDir lebih dulu
/// (bundled Android), lalu `php` di PATH (host/test).
pub fn find_php_binary(native_lib_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = native_lib_dir {
        let cand = dir.join("libphp.so");
        if cand.is_file() {
            return Some(cand);
        }
    }
    crate::runtime::find_in_path("php")
}

/// true bila binary berasal dari bundle (libphp.so), bukan PATH.
pub fn is_bundled_php(binary: &Path, native_lib_dir: Option<&Path>) -> bool {
    match native_lib_dir {
        Some(dir) => binary == dir.join("libphp.so"),
        None => false,
    }
}

/// Argumen untuk menjalankan PHP built-in server:
/// `php -S host:port -t <docroot> -c <php.ini>`
pub fn build_php_server_args(host: &str, port: u16, docroot: &Path, ini_path: &Path) -> Vec<String> {
    vec![
        "-S".to_string(),
        format!("{}:{}", host, port),
        "-t".to_string(),
        docroot.to_string_lossy().to_string(),
        "-c".to_string(),
        ini_path.to_string_lossy().to_string(),
    ]
}

/// Parse versi dari output `php -v`, mis. "PHP 8.3.14 (cli) ..." -> "8.3.14".
pub fn parse_php_version(output: &str) -> Option<String> {
    let first = output.lines().next()?.trim();
    let mut parts = first.split_whitespace();
    if parts.next()? != "PHP" {
        return None;
    }
    let v = parts.next()?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Parse daftar ekstensi dari output `php -m` (bagian [PHP Modules] saja;
/// [Zend Modules] tidak dihitung sebagai ekstensi PHP).
pub fn parse_php_modules(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_php_modules = false;
    for line in output.lines() {
        let t = line.trim();
        if t == "[PHP Modules]" {
            in_php_modules = true;
            continue;
        }
        if t == "[Zend Modules]" {
            break;
        }
        if in_php_modules && !t.is_empty() {
            out.push(t.to_string());
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhpSettings {
    pub memory_limit: String,
    pub upload_max_filesize: String,
    pub post_max_size: String,
    pub max_execution_time: i64,
    pub date_timezone: String,
    pub display_errors: bool,
}

impl Default for PhpSettings {
    fn default() -> Self {
        Self {
            memory_limit: "128M".to_string(),
            upload_max_filesize: "32M".to_string(),
            post_max_size: "32M".to_string(),
            max_execution_time: 30,
            date_timezone: "UTC".to_string(),
            display_errors: false,
        }
    }
}

fn valid_size_value(s: &str, allow_minus_one: bool) -> bool {
    let t = s.trim();
    if allow_minus_one && t == "-1" {
        return true;
    }
    if t.is_empty() || t.len() > 16 {
        return false;
    }
    let (digits, _suffix) = match t.chars().last() {
        Some(c) if c == 'K' || c == 'M' || c == 'G' || c == 'k' || c == 'm' || c == 'g' => {
            (&t[..t.len() - 1], true)
        }
        _ => (t, false),
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

impl PhpSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_size_value(&self.memory_limit, true) {
            return Err("memory_limit tidak valid (contoh: 128M, 512M, -1)".to_string());
        }
        if !valid_size_value(&self.upload_max_filesize, false) {
            return Err("upload_max_filesize tidak valid (contoh: 32M)".to_string());
        }
        if !valid_size_value(&self.post_max_size, false) {
            return Err("post_max_size tidak valid (contoh: 32M)".to_string());
        }
        if !(0..=3600).contains(&self.max_execution_time) {
            return Err("max_execution_time harus antara 0 dan 3600 detik".to_string());
        }
        let tz = self.date_timezone.trim();
        if tz.is_empty()
            || tz.len() > 64
            || !tz.chars().all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-' || c == '+' || c == '.')
        {
            return Err("date_timezone tidak valid (contoh: UTC, Asia/Jakarta)".to_string());
        }
        Ok(())
    }
}

pub fn php_ini_path(data_dir: &Path) -> PathBuf {
    data_dir.join("php.ini")
}

pub fn php_tmp_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("php-tmp")
}

pub fn php_sessions_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("php-sessions")
}

/// Hasilkan isi php.ini dari pengaturan. Folder tmp/session diarahkan ke
/// data dir aplikasi karena Android tidak menyediakan /tmp yang bisa
/// ditulis aplikasi.
pub fn generate_php_ini(settings: &PhpSettings, data_dir: &Path) -> String {
    let tmp = php_tmp_dir(data_dir).to_string_lossy().to_string();
    let sessions = php_sessions_dir(data_dir).to_string_lossy().to_string();
    format!(
        "; Dihasilkan otomatis oleh WebForge Mobile — jangan edit manual.\n\
         [PHP]\n\
         memory_limit = {}\n\
         upload_max_filesize = {}\n\
         post_max_size = {}\n\
         max_execution_time = {}\n\
         date.timezone = {}\n\
         display_errors = {}\n\
         display_startup_errors = Off\n\
         log_errors = On\n\
         error_reporting = E_ALL\n\
         expose_php = Off\n\
         sys_temp_dir = \"{}\"\n\
         upload_tmp_dir = \"{}\"\n\
         session.save_path = \"{}\"\n",
        settings.memory_limit.trim(),
        settings.upload_max_filesize.trim(),
        settings.post_max_size.trim(),
        settings.max_execution_time,
        settings.date_timezone.trim(),
        if settings.display_errors { "On" } else { "Off" },
        tmp,
        tmp,
        sessions,
    )
}

/// Tulis php.ini ke data dir (beserta folder tmp/session). Mengembalikan
/// path php.ini yang ditulis.
pub fn write_php_ini(data_dir: &Path, settings: &PhpSettings) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(php_tmp_dir(data_dir))?;
    std::fs::create_dir_all(php_sessions_dir(data_dir))?;
    let path = php_ini_path(data_dir);
    std::fs::write(&path, generate_php_ini(settings, data_dir))?;
    Ok(path)
}

async fn get_setting_raw(pool: &sqlx::SqlitePool, key: &str) -> Option<String> {
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

pub async fn load_php_settings(pool: &sqlx::SqlitePool) -> PhpSettings {
    let mut s = PhpSettings::default();
    if let Some(v) = get_setting_raw(pool, "php.memory_limit").await {
        s.memory_limit = parse_setting_string(&v);
    }
    if let Some(v) = get_setting_raw(pool, "php.upload_max_filesize").await {
        s.upload_max_filesize = parse_setting_string(&v);
    }
    if let Some(v) = get_setting_raw(pool, "php.post_max_size").await {
        s.post_max_size = parse_setting_string(&v);
    }
    if let Some(v) = get_setting_raw(pool, "php.max_execution_time").await {
        if let Ok(n) = serde_json::from_str::<i64>(&v).or_else(|_| v.trim().parse::<i64>()) {
            s.max_execution_time = n;
        }
    }
    if let Some(v) = get_setting_raw(pool, "php.date_timezone").await {
        s.date_timezone = parse_setting_string(&v);
    }
    if let Some(v) = get_setting_raw(pool, "php.display_errors").await {
        if let Ok(b) = serde_json::from_str::<bool>(&v).or_else(|_| v.trim().parse::<bool>()) {
            s.display_errors = b;
        }
    }
    s
}

pub async fn save_php_settings(pool: &sqlx::SqlitePool, settings: &PhpSettings) -> Result<(), sqlx::Error> {
    let now = crate::util::now_iso();
    let values: [(&str, String); 6] = [
        ("php.memory_limit", serde_json::to_string(&settings.memory_limit).unwrap()),
        ("php.upload_max_filesize", serde_json::to_string(&settings.upload_max_filesize).unwrap()),
        ("php.post_max_size", serde_json::to_string(&settings.post_max_size).unwrap()),
        ("php.max_execution_time", settings.max_execution_time.to_string()),
        ("php.date_timezone", serde_json::to_string(&settings.date_timezone).unwrap()),
        ("php.display_errors", settings.display_errors.to_string()),
    ];
    for (key, value) in values {
        sqlx::query(
            "INSERT INTO user_settings (key, value, updated_at) VALUES (?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(&value)
        .bind(&now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct PhpInfo {
    pub available: bool,
    pub version: Option<String>,
    pub extensions: Vec<String>,
    pub binary_path: Option<String>,
    pub error: Option<String>,
}

impl PhpInfo {
    pub fn unavailable(error: String) -> Self {
        Self {
            available: false,
            version: None,
            extensions: Vec::new(),
            binary_path: None,
            error: Some(error),
        }
    }
}

async fn run_php_capture(binary: &Path, arg: &str) -> Result<std::process::Output, String> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg(arg);
    let home = crate::state::data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("webforge-probe"));
    crate::util::prepare_spawn(
        &mut cmd,
        crate::state::native_lib_dir().as_deref(),
        &home,
        Some(&home),
    );
    match tokio::time::timeout(crate::runtime::PROBE_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(format!("Gagal menjalankan PHP: {}", e)),
        Err(_) => Err(format!(
            "PHP tidak merespons dalam {} detik",
            crate::runtime::PROBE_TIMEOUT.as_secs()
        )),
    }
}

/// Probe PHP nyata: jalankan `php -v` dan `php -m` pada binary yang ditemukan.
pub async fn probe_php_info(binary: Option<PathBuf>) -> PhpInfo {
    let Some(binary) = binary else {
        return PhpInfo::unavailable(
            "Binary PHP tidak ditemukan: libphp.so tidak ada di nativeLibraryDir dan php tidak ada di PATH".to_string(),
        );
    };
    let binary_path = binary.to_string_lossy().to_string();
    let v_out = match run_php_capture(&binary, "-v").await {
        Ok(o) => o,
        Err(e) => {
            return PhpInfo {
                available: false,
                version: None,
                extensions: Vec::new(),
                binary_path: Some(binary_path),
                error: Some(e),
            }
        }
    };
    let v_text = String::from_utf8_lossy(&v_out.stdout).to_string();
    let version = parse_php_version(&v_text);
    if !v_out.status.success() || version.is_none() {
        let stderr = String::from_utf8_lossy(&v_out.stderr).trim().to_string();
        return PhpInfo {
            available: false,
            version: None,
            extensions: Vec::new(),
            binary_path: Some(binary_path),
            error: Some(if stderr.is_empty() {
                "php -v gagal atau output tidak dikenal".to_string()
            } else {
                format!("php -v gagal: {}", stderr.chars().take(300).collect::<String>())
            }),
        };
    }
    let extensions = match run_php_capture(&binary, "-m").await {
        Ok(o) if o.status.success() => parse_php_modules(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    };
    PhpInfo {
        available: true,
        version,
        extensions,
        binary_path: Some(binary_path),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_generation_from_settings() {
        let s = PhpSettings {
            memory_limit: "256M".to_string(),
            upload_max_filesize: "64M".to_string(),
            post_max_size: "64M".to_string(),
            max_execution_time: 60,
            date_timezone: "Asia/Jakarta".to_string(),
            display_errors: true,
        };
        let ini = generate_php_ini(&s, Path::new("/data/webforge"));
        assert!(ini.contains("memory_limit = 256M"));
        assert!(ini.contains("upload_max_filesize = 64M"));
        assert!(ini.contains("post_max_size = 64M"));
        assert!(ini.contains("max_execution_time = 60"));
        assert!(ini.contains("date.timezone = Asia/Jakarta"));
        assert!(ini.contains("display_errors = On"));
        assert!(ini.contains("session.save_path = \"/data/webforge/php-sessions\""));
        assert!(ini.contains("sys_temp_dir = \"/data/webforge/php-tmp\""));
    }

    #[test]
    fn ini_display_errors_off_by_default() {
        let ini = generate_php_ini(&PhpSettings::default(), Path::new("/data/webforge"));
        assert!(ini.contains("display_errors = Off"));
        assert!(ini.contains("memory_limit = 128M"));
    }

    #[test]
    fn server_args_built_exactly() {
        let args = build_php_server_args(
            "0.0.0.0",
            8080,
            Path::new("/data/workspace/site"),
            Path::new("/data/webforge/php.ini"),
        );
        assert_eq!(
            args,
            vec![
                "-S".to_string(),
                "0.0.0.0:8080".to_string(),
                "-t".to_string(),
                "/data/workspace/site".to_string(),
                "-c".to_string(),
                "/data/webforge/php.ini".to_string(),
            ]
        );
    }

    #[test]
    fn parse_modules_excludes_zend_section() {
        let out = "[PHP Modules]\nbcmath\nCore\njson\nmbstring\n\n[Zend Modules]\nZend OPcache\n";
        assert_eq!(parse_php_modules(out), vec!["bcmath", "Core", "json", "mbstring"]);
    }

    #[test]
    fn parse_modules_empty_without_header() {
        assert!(parse_php_modules("bukan output php -m").is_empty());
    }

    #[test]
    fn parse_version_from_php_v() {
        let out = "PHP 8.3.14 (cli) (built: Nov 20 2024 10:00:00) (NTS)\nCopyright (c) The PHP Group\n";
        assert_eq!(parse_php_version(out).as_deref(), Some("8.3.14"));
        assert!(parse_php_version("bukan php").is_none());
    }

    #[test]
    fn restart_budget_three_attempts() {
        assert_eq!(restart_delay(0), Some(Duration::from_secs(2)));
        assert_eq!(restart_delay(1), Some(Duration::from_secs(5)));
        assert_eq!(restart_delay(2), Some(Duration::from_secs(10)));
        assert_eq!(restart_delay(3), None);
        assert_eq!(restart_delay(99), None);
    }

    #[test]
    fn settings_validation() {
        assert!(PhpSettings::default().validate().is_ok());
        let mut s = PhpSettings::default();
        s.memory_limit = "-1".to_string();
        assert!(s.validate().is_ok());
        s.memory_limit = "banyak".to_string();
        assert!(s.validate().is_err());
        s.memory_limit = "128M".to_string();
        s.max_execution_time = -5;
        assert!(s.validate().is_err());
        s.max_execution_time = 30;
        s.date_timezone = "Asia/Jakarta; rm -rf".to_string();
        assert!(s.validate().is_err());
    }

    #[test]
    fn find_binary_prefers_bundled_libphp() {
        let dir = std::env::temp_dir().join(format!("wf-php-test-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("libphp.so"), b"fake").unwrap();
        assert_eq!(find_php_binary(Some(&dir)), Some(dir.join("libphp.so")));
        assert!(is_bundled_php(&dir.join("libphp.so"), Some(&dir)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
