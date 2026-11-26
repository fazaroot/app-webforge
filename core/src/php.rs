//! PHP runtime: deteksi binary nyata multi-versi, pengaturan php.ini,
//! probe info, konfigurasi PHP-FPM, Composer, dan logika restart server.
//!
//! Binary PHP di Android adalah `libphpXX.so` (PHP CLI hasil cross-compile,
//! dikemas sebagai .so agar sah dieksekusi dari nativeLibraryDir — W^X)
//! dan `libphpfpmXX.so` untuk PHP-FPM. Versi terbundel v0.5.0: 8.2.36,
//! 8.3.37, 8.4.28. Fallback: `libphp.so` lama dan `php`/`php-fpm` dari
//! PATH terdeteksi sebagai versinya lewat probe `php -v` sungguhan.
//! Tidak ada daftar ekstensi atau versi yang dikarang: semua berasal dari
//! menjalankan binary-nya.
//!
//! Versi default bila pengguna tidak memilih: PHP 8.3 bila tersedia
//! (versi stabil yang sudah teruji di perangkat), selain itu versi
//! tertinggi yang tersedia.

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

// ---------------------------------------------------------------------------
// Versi PHP terbundel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhpVersionSpec {
    /// Versi mayor.minor, mis. "8.3".
    pub version: &'static str,
    /// Versi patch terbundel, mis. "8.3.37".
    pub patch_version: &'static str,
    /// Nama binary CLI terbundel di nativeLibraryDir.
    pub cli_binary: &'static str,
    /// Nama binary PHP-FPM terbundel di nativeLibraryDir.
    pub fpm_binary: &'static str,
}

pub const PHP_VERSIONS: [PhpVersionSpec; 3] = [
    PhpVersionSpec {
        version: "8.2",
        patch_version: "8.2.36",
        cli_binary: "libphp82.so",
        fpm_binary: "libphpfpm82.so",
    },
    PhpVersionSpec {
        version: "8.3",
        patch_version: "8.3.37",
        cli_binary: "libphp83.so",
        fpm_binary: "libphpfpm83.so",
    },
    PhpVersionSpec {
        version: "8.4",
        patch_version: "8.4.28",
        cli_binary: "libphp84.so",
        fpm_binary: "libphpfpm84.so",
    },
];

/// Versi default yang dipilih bila tersedia (stabil, teruji di perangkat).
pub const DEFAULT_PHP_VERSION: &str = "8.3";

pub fn php_version_spec(version: &str) -> Option<&'static PhpVersionSpec> {
    PHP_VERSIONS.iter().find(|s| s.version == version)
}

/// Normalisasi masukan versi ("8.3", "8.3.37", "php8.3") menjadi
/// "mayor.minor". None bila format tidak dikenal.
pub fn normalize_php_version(input: &str) -> Option<String> {
    let t = input.trim().trim_start_matches("php").trim_start_matches('v');
    let mut parts = t.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    if let Some(patch) = parts.next() {
        patch.parse::<u32>().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{}.{}", major, minor))
}

/// Validasi pilihan versi PHP pengguna: None/kosong berarti "ikuti
/// default". Nilai lain wajib salah satu versi terbundel.
pub fn validate_php_version_choice(input: Option<&str>) -> Result<Option<String>, String> {
    match input.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(raw) => {
            let normalized = normalize_php_version(raw).ok_or_else(|| {
                format!("php_version '{}' tidak valid (contoh: 8.2, 8.3, 8.4)", raw)
            })?;
            if php_version_spec(&normalized).is_none() {
                return Err(format!(
                    "Versi PHP {} tidak terbundel; pilihan tersedia: {}",
                    normalized,
                    PHP_VERSIONS
                        .iter()
                        .map(|s| s.version)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(Some(normalized))
        }
    }
}

fn version_key(version: &str) -> (u32, u32) {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    (major, minor)
}

/// Aturan versi default: 8.3 bila tersedia, selain itu versi tertinggi
/// yang tersedia. None bila tidak ada versi yang tersedia.
pub fn default_php_version(versions: &[PhpVersionInfo]) -> Option<String> {
    if versions
        .iter()
        .any(|v| v.available && v.version == DEFAULT_PHP_VERSION)
    {
        return Some(DEFAULT_PHP_VERSION.to_string());
    }
    versions
        .iter()
        .filter(|v| v.available)
        .max_by_key(|v| version_key(&v.version))
        .map(|v| v.version.clone())
}

// ---------------------------------------------------------------------------
// Penemuan binary
// ---------------------------------------------------------------------------

fn bundled_file(native_lib_dir: Option<&Path>, name: &str) -> Option<PathBuf> {
    let cand = native_lib_dir?.join(name);
    if cand.is_file() {
        Some(cand)
    } else {
        None
    }
}

/// Binary CLI terbundel untuk satu versi persis (tanpa fallback PATH,
/// karena versi binary PATH belum diketahui sebelum probe).
pub fn find_php_binary_for_version(native_lib_dir: Option<&Path>, version: &str) -> Option<PathBuf> {
    let spec = php_version_spec(version)?;
    bundled_file(native_lib_dir, spec.cli_binary)
}

/// Binary FPM terbundel untuk satu versi persis.
pub fn find_fpm_binary_for_version(native_lib_dir: Option<&Path>, version: &str) -> Option<PathBuf> {
    let spec = php_version_spec(version)?;
    bundled_file(native_lib_dir, spec.fpm_binary)
}

/// Cari binary PHP nyata untuk jalur lama/default: binary terbundel
/// versi default (8.3) lebih dulu, lalu versi terbundel lain, lalu
/// `libphp.so` lama, lalu `php` di PATH.
pub fn find_php_binary(native_lib_dir: Option<&Path>) -> Option<PathBuf> {
    for spec in [
        php_version_spec(DEFAULT_PHP_VERSION),
        php_version_spec("8.4"),
        php_version_spec("8.2"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(p) = bundled_file(native_lib_dir, spec.cli_binary) {
            return Some(p);
        }
    }
    if let Some(p) = bundled_file(native_lib_dir, "libphp.so") {
        return Some(p);
    }
    crate::runtime::find_in_path("php")
}

/// Cari binary PHP-FPM nyata (versi default lebih dulu), lalu PATH.
pub fn find_fpm_binary(native_lib_dir: Option<&Path>) -> Option<PathBuf> {
    for spec in [
        php_version_spec(DEFAULT_PHP_VERSION),
        php_version_spec("8.4"),
        php_version_spec("8.2"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(p) = bundled_file(native_lib_dir, spec.fpm_binary) {
            return Some(p);
        }
    }
    crate::runtime::find_in_path("php-fpm")
}

/// true bila binary berasal dari bundle (libphp*.so), bukan PATH.
pub fn is_bundled_php(binary: &Path, native_lib_dir: Option<&Path>) -> bool {
    let Some(dir) = native_lib_dir else {
        return false;
    };
    if binary.parent() != Some(dir) {
        return false;
    }
    match binary.file_name().and_then(|n| n.to_str()) {
        Some(name) => {
            name == "libphp.so" || PHP_VERSIONS.iter().any(|s| s.cli_binary == name)
        }
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

// ---------------------------------------------------------------------------
// Pengaturan php.ini
// ---------------------------------------------------------------------------

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

/// Path berkas CA bundle terbundel bila ada (data/assets/cacert.pem).
pub fn cacert_path(data_dir: &Path) -> PathBuf {
    data_dir.join("assets").join("cacert.pem")
}

/// Path data ICU terbundel bila ada (data/assets/icudt74l.dat).
pub fn icu_data_path(data_dir: &Path) -> PathBuf {
    data_dir.join("assets").join("icudt74l.dat")
}

/// Path binary shim sendmail: `libsendmail.so` di nativeLibraryDir,
/// atau `webforge-sendmail` di PATH (host/test).
pub fn find_sendmail_binary(native_lib_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = bundled_file(native_lib_dir, "libsendmail.so") {
        return Some(p);
    }
    crate::runtime::find_in_path("webforge-sendmail")
}

/// Perintah sendmail_path untuk php.ini/FPM: shim WebForge dengan
/// konfigurasi SMTP di data/mail/smtp.json. Di Android (native dir
/// diketahui) path shim selalu ditulis agar mail() gagal jujur lewat
/// shim bila SMTP belum dikonfigurasi, bukan diam-diam ke sendmail
/// sistem yang tidak ada.
pub fn sendmail_command(data_dir: &Path, native_lib_dir: Option<&Path>) -> Option<String> {
    let shim = match find_sendmail_binary(native_lib_dir) {
        Some(p) => p,
        None => native_lib_dir?.join("libsendmail.so"),
    };
    let config = crate::routes::mail::smtp_json_path(data_dir);
    Some(format!(
        "{} --config {} -t -i",
        shim.to_string_lossy(),
        config.to_string_lossy()
    ))
}

/// Hasilkan isi php.ini dari pengaturan. Folder tmp/session diarahkan ke
/// data dir aplikasi karena Android tidak menyediakan /tmp yang bisa
/// ditulis aplikasi. sendmail_path menunjuk shim WebForge; CA bundle
/// diarahkan ke data/assets/cacert.pem bila berkasnya ada.
pub fn generate_php_ini_full(
    settings: &PhpSettings,
    data_dir: &Path,
    native_lib_dir: Option<&Path>,
) -> String {
    let tmp = php_tmp_dir(data_dir).to_string_lossy().to_string();
    let sessions = php_sessions_dir(data_dir).to_string_lossy().to_string();
    let mut ini = format!(
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
    );
    if let Some(cmd) = sendmail_command(data_dir, native_lib_dir) {
        ini.push_str(&format!("sendmail_path = \"{}\"\n", cmd.replace('"', "\\\"")));
    }
    let cacert = cacert_path(data_dir);
    if cacert.is_file() {
        let p = cacert.to_string_lossy().to_string();
        ini.push_str(&format!(
            "openssl.cafile = \"{}\"\ncurl.cainfo = \"{}\"\n",
            p, p
        ));
    }
    ini
}

/// Kompatibilitas: php.ini dengan nativeLibraryDir dari state global.
pub fn generate_php_ini(settings: &PhpSettings, data_dir: &Path) -> String {
    generate_php_ini_full(settings, data_dir, crate::state::native_lib_dir().as_deref())
}

/// Tulis php.ini ke data dir (beserta folder tmp/session) dan, bila
/// `smtp` diberikan, tulis data/mail/smtp.json (0600) dari konfigurasi
/// SMTP terdekripsi. Bila `smtp` None, smtp.json yang ada dihapus agar
/// shim sendmail gagal jujur (SMTP belum dikonfigurasi).
pub fn write_php_ini_with_smtp(
    data_dir: &Path,
    settings: &PhpSettings,
    smtp: Option<&crate::routes::mail::SmtpConfig>,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(php_tmp_dir(data_dir))?;
    std::fs::create_dir_all(php_sessions_dir(data_dir))?;
    let path = php_ini_path(data_dir);
    std::fs::write(&path, generate_php_ini(settings, data_dir))?;
    crate::routes::mail::write_smtp_json(data_dir, smtp)?;
    Ok(path)
}

/// Tulis php.ini saja (tanpa menyentuh smtp.json). Jalur produksi
/// memakai [`write_php_ini_with_smtp`] agar smtp.json selalu sinkron.
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

// ---------------------------------------------------------------------------
// Probe multi-versi + Composer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PhpVersionInfo {
    pub version: String,
    pub patch_version: Option<String>,
    pub available: bool,
    pub fpm_available: bool,
    pub extensions: Vec<String>,
    pub binary_path: Option<String>,
    pub fpm_binary_path: Option<String>,
    pub error: Option<String>,
}

impl PhpVersionInfo {
    fn unavailable(version: &str, error: String) -> Self {
        Self {
            version: version.to_string(),
            patch_version: None,
            available: false,
            fpm_available: false,
            extensions: Vec::new(),
            binary_path: None,
            fpm_binary_path: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComposerInfo {
    pub available: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub error: Option<String>,
}

impl ComposerInfo {
    pub fn unavailable(error: String) -> Self {
        Self {
            available: false,
            version: None,
            path: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PhpInfo {
    pub available: bool,
    pub version: Option<String>,
    pub extensions: Vec<String>,
    pub binary_path: Option<String>,
    pub error: Option<String>,
    pub selected_version: Option<String>,
    pub versions: Vec<PhpVersionInfo>,
    pub composer: ComposerInfo,
}

impl PhpInfo {
    /// Bangun info dari hasil probe semua versi. Field lama
    /// (available/version/extensions/binary_path/error) mencerminkan
    /// versi terpilih: `requested` bila diberikan, selain itu aturan
    /// default (8.3 bila tersedia, else tertinggi tersedia).
    pub fn from_versions(
        versions: Vec<PhpVersionInfo>,
        composer: ComposerInfo,
        requested: Option<&str>,
    ) -> Self {
        let selected = requested
            .and_then(normalize_php_version)
            .or_else(|| default_php_version(&versions));
        let entry = selected
            .as_ref()
            .and_then(|sel| versions.iter().find(|v| &v.version == sel));
        match entry {
            Some(e) => Self {
                available: e.available,
                version: e.patch_version.clone(),
                extensions: e.extensions.clone(),
                binary_path: e.binary_path.clone(),
                error: e.error.clone(),
                selected_version: selected,
                versions,
                composer,
            },
            None => Self {
                available: false,
                version: None,
                extensions: Vec::new(),
                binary_path: None,
                error: Some(
                    "Binary PHP tidak ditemukan: libphp82.so/libphp83.so/libphp84.so/libphp.so tidak ada di nativeLibraryDir dan php tidak ada di PATH"
                        .to_string(),
                ),
                selected_version: selected,
                versions,
                composer,
            },
        }
    }

    /// Salinan dengan pilihan versi lain (untuk cache: probe sekali,
    /// pilihan per request).
    pub fn select(&self, requested: Option<&str>) -> Self {
        Self::from_versions(self.versions.clone(), self.composer.clone(), requested)
    }
}

async fn run_capture(binary: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(args);
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
        Ok(Err(e)) => Err(format!("Gagal menjalankan {}: {}", binary.display(), e)),
        Err(_) => Err(format!(
            "{} tidak merespons dalam {} detik",
            binary.display(),
            crate::runtime::PROBE_TIMEOUT.as_secs()
        )),
    }
}

/// Probe CLI satu binary: (patch_version, extensions) atau error jujur.
async fn probe_cli(binary: &Path) -> Result<(String, Vec<String>), String> {
    let v_out = run_capture(binary, &["-v"]).await?;
    let v_text = String::from_utf8_lossy(&v_out.stdout).to_string();
    let version = parse_php_version(&v_text);
    if !v_out.status.success() || version.is_none() {
        let stderr = String::from_utf8_lossy(&v_out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "php -v gagal atau output tidak dikenal".to_string()
        } else {
            format!("php -v gagal: {}", stderr.chars().take(300).collect::<String>())
        });
    }
    let extensions = match run_capture(binary, &["-m"]).await {
        Ok(o) if o.status.success() => parse_php_modules(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    };
    Ok((version.unwrap(), extensions))
}

/// Probe FPM satu binary lewat `php-fpm -v` ("PHP 8.3.37 (fpm-fcgi)").
async fn probe_fpm(binary: &Path) -> Result<String, String> {
    let out = run_capture(binary, &["-v"]).await?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    match parse_php_version(&text) {
        Some(v) if out.status.success() => Ok(v),
        _ => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                "php-fpm -v gagal atau output tidak dikenal".to_string()
            } else {
                format!("php-fpm -v gagal: {}", stderr.chars().take(300).collect::<String>())
            })
        }
    }
}

/// Probe semua versi PHP: binary terbundel per versi, lalu fallback
/// `libphp.so` lama dan `php`/`php-fpm` PATH yang terdeteksi sebagai
/// versinya. Urutan hasil: 8.2, 8.3, 8.4, lalu versi lain terdeteksi.
pub async fn probe_all_php_versions(native_lib_dir: Option<&Path>) -> Vec<PhpVersionInfo> {
    let mut versions: Vec<PhpVersionInfo> = Vec::new();

    for spec in PHP_VERSIONS.iter() {
        let mut info = PhpVersionInfo::unavailable(
            spec.version,
            format!(
                "Binary PHP {} tidak ditemukan: {} tidak ada di nativeLibraryDir dan php {} tidak ada di PATH",
                spec.version, spec.cli_binary, spec.version
            ),
        );
        if let Some(cli) = bundled_file(native_lib_dir, spec.cli_binary) {
            info.binary_path = Some(cli.to_string_lossy().to_string());
            match probe_cli(&cli).await {
                Ok((patch, extensions)) => {
                    info.available = true;
                    info.patch_version = Some(patch);
                    info.extensions = extensions;
                    info.error = None;
                }
                Err(e) => info.error = Some(e),
            }
        }
        if let Some(fpm) = bundled_file(native_lib_dir, spec.fpm_binary) {
            info.fpm_binary_path = Some(fpm.to_string_lossy().to_string());
            // FPM tersedia bila binary-nya ada; probe -v dicoba dan
            // kegagalannya dicatat sebagai catatan error.
            info.fpm_available = true;
            if let Err(e) = probe_fpm(&fpm).await {
                let note = format!("Probe PHP-FPM {}: {}", spec.version, e);
                info.error = Some(match info.error.take() {
                    Some(prev) => format!("{}; {}", prev, note),
                    None => note,
                });
            }
        }
        versions.push(info);
    }

    // Fallback CLI: libphp.so lama, lalu php di PATH. Keduanya
    // terdeteksi sebagai versinya dan mengisi slot yang belum tersedia.
    let mut cli_fallbacks: Vec<PathBuf> = Vec::new();
    if let Some(p) = bundled_file(native_lib_dir, "libphp.so") {
        cli_fallbacks.push(p);
    }
    if let Some(p) = crate::runtime::find_in_path("php") {
        cli_fallbacks.push(p);
    }
    for cli in cli_fallbacks {
        if let Ok((patch, extensions)) = probe_cli(&cli).await {
            if let Some(major_minor) = normalize_php_version(&patch) {
                match versions.iter_mut().find(|v| v.version == major_minor) {
                    Some(slot) if !slot.available => {
                        slot.available = true;
                        slot.patch_version = Some(patch);
                        slot.extensions = extensions;
                        slot.binary_path = Some(cli.to_string_lossy().to_string());
                        slot.error = None;
                    }
                    Some(_) => {}
                    None => versions.push(PhpVersionInfo {
                        version: major_minor,
                        patch_version: Some(patch),
                        available: true,
                        fpm_available: false,
                        extensions,
                        binary_path: Some(cli.to_string_lossy().to_string()),
                        fpm_binary_path: None,
                        error: None,
                    }),
                }
            }
        }
    }

    // Fallback FPM dari PATH.
    if let Some(fpm) = crate::runtime::find_in_path("php-fpm") {
        if let Ok(patch) = probe_fpm(&fpm).await {
            if let Some(major_minor) = normalize_php_version(&patch) {
                if let Some(slot) = versions.iter_mut().find(|v| v.version == major_minor) {
                    if !slot.fpm_available {
                        slot.fpm_available = true;
                        slot.fpm_binary_path = Some(fpm.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    versions
}

/// Hasil resolusi PHP untuk menjalankan server/CLI.
#[derive(Debug, Clone)]
pub struct ResolvedPhp {
    pub version: String,
    pub patch_version: Option<String>,
    pub binary: PathBuf,
    pub fpm_binary: Option<PathBuf>,
}

/// Resolusi versi PHP yang akan dipakai: `requested` bila diberikan
/// (wajib tersedia), selain itu aturan default. Error jujur berisi
/// daftar versi yang tersedia.
pub async fn resolve_php(
    native_lib_dir: Option<&Path>,
    requested: Option<&str>,
) -> Result<ResolvedPhp, String> {
    let versions = probe_all_php_versions(native_lib_dir).await;
    let available_list = versions
        .iter()
        .filter(|v| v.available)
        .map(|v| v.version.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let chosen = match requested {
        Some(raw) => {
            let normalized = normalize_php_version(raw)
                .ok_or_else(|| format!("Versi PHP '{}' tidak valid", raw))?;
            versions
                .iter()
                .find(|v| v.version == normalized)
                .ok_or_else(|| {
                    format!(
                        "Versi PHP {} tidak dikenal; versi tersedia: {}",
                        normalized,
                        if available_list.is_empty() { "tidak ada".to_string() } else { available_list.clone() }
                    )
                })?
        }
        None => {
            let default = default_php_version(&versions).ok_or_else(|| {
                "PHP tidak tersedia: tidak ada binary PHP yang terdeteksi (libphp82.so/libphp83.so/libphp84.so/libphp.so di nativeLibraryDir atau php di PATH)".to_string()
            })?;
            versions.iter().find(|v| v.version == default).unwrap()
        }
    };
    if !chosen.available {
        return Err(format!(
            "PHP {} tidak tersedia: {}. Versi tersedia: {}",
            chosen.version,
            chosen.error.clone().unwrap_or_else(|| "binary tidak ditemukan".to_string()),
            if available_list.is_empty() { "tidak ada".to_string() } else { available_list }
        ));
    }
    let binary = chosen
        .binary_path
        .clone()
        .map(PathBuf::from)
        .ok_or_else(|| format!("PHP {} tersedia tetapi path binary kosong", chosen.version))?;
    Ok(ResolvedPhp {
        version: chosen.version.clone(),
        patch_version: chosen.patch_version.clone(),
        binary,
        fpm_binary: chosen.fpm_binary_path.clone().map(PathBuf::from),
    })
}

/// Path composer.phar terbundel (data/assets/composer.phar).
pub fn composer_phar_path(data_dir: &Path) -> PathBuf {
    data_dir.join("assets").join("composer.phar")
}

/// Parse versi dari output `composer --version`.
pub fn parse_composer_version(output: &str) -> Option<String> {
    for line in output.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Composer version ") {
            let v = rest.split_whitespace().next()?.to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Probe Composer nyata: jalankan `php composer.phar --version`, atau
/// `composer --version` dari PATH sebagai fallback host.
pub async fn probe_composer(data_dir: &Path, php_binary: Option<&Path>) -> ComposerInfo {
    let phar = composer_phar_path(data_dir);
    if phar.is_file() {
        let Some(php) = php_binary else {
            return ComposerInfo {
                available: false,
                version: None,
                path: Some(phar.to_string_lossy().to_string()),
                error: Some("composer.phar ada tetapi PHP tidak tersedia untuk menjalankannya".to_string()),
            };
        };
        let phar_str = phar.to_string_lossy().to_string();
        return match run_capture(php, &[&phar_str, "--version"]).await {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                match parse_composer_version(&text) {
                    Some(v) => ComposerInfo {
                        available: true,
                        version: Some(v),
                        path: Some(phar_str),
                        error: None,
                    },
                    None => ComposerInfo {
                        available: false,
                        version: None,
                        path: Some(phar_str),
                        error: Some("composer --version gagal atau output tidak dikenal".to_string()),
                    },
                }
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                ComposerInfo {
                    available: false,
                    version: None,
                    path: Some(phar_str),
                    error: Some(if stderr.is_empty() {
                        "composer --version gagal".to_string()
                    } else {
                        format!("composer --version gagal: {}", stderr.chars().take(300).collect::<String>())
                    }),
                }
            }
            Err(e) => ComposerInfo {
                available: false,
                version: None,
                path: Some(phar_str),
                error: Some(e),
            },
        };
    }
    if let Some(composer) = crate::runtime::find_in_path("composer") {
        let path_str = composer.to_string_lossy().to_string();
        return match run_capture(&composer, &["--version"]).await {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                match parse_composer_version(&text) {
                    Some(v) => ComposerInfo {
                        available: true,
                        version: Some(v),
                        path: Some(path_str),
                        error: None,
                    },
                    None => ComposerInfo {
                        available: false,
                        version: None,
                        path: Some(path_str),
                        error: Some("composer --version gagal atau output tidak dikenal".to_string()),
                    },
                }
            }
            Ok(_) => ComposerInfo {
                available: false,
                version: None,
                path: Some(path_str),
                error: Some("composer --version gagal".to_string()),
            },
            Err(e) => ComposerInfo {
                available: false,
                version: None,
                path: Some(path_str),
                error: Some(e),
            },
        };
    }
    ComposerInfo::unavailable(
        "Composer tidak tersedia: composer.phar tidak ditemukan di data/assets dan composer tidak ada di PATH".to_string(),
    )
}

// ---------------------------------------------------------------------------
// PHP-FPM
// ---------------------------------------------------------------------------

/// Folder home FPM satu server: <data_dir>/servers/<id>/php-fpm.
pub fn fpm_home(data_dir: &Path, server_id: &str) -> PathBuf {
    crate::webservers::server_home(data_dir, server_id, "php-fpm")
}

pub fn fpm_config_path(data_dir: &Path, server_id: &str) -> PathBuf {
    fpm_home(data_dir, server_id).join("php-fpm.conf")
}

pub fn fpm_error_log_path(data_dir: &Path, server_id: &str) -> PathBuf {
    fpm_home(data_dir, server_id).join("logs").join("fpm-error.log")
}

#[derive(Debug, Clone)]
pub struct FpmConfigInput {
    pub data_dir: PathBuf,
    pub server_id: String,
    pub backend_port: u16,
    pub native_lib_dir: Option<PathBuf>,
}

/// Environment proses FPM: TMPDIR/PHPRC ke data dir, ICU_DATA ke
/// data/assets bila icudt74l.dat ada.
pub fn fpm_envs(data_dir: &Path) -> Vec<(String, String)> {
    let mut envs = vec![
        ("HOME".to_string(), data_dir.to_string_lossy().to_string()),
        ("TMPDIR".to_string(), php_tmp_dir(data_dir).to_string_lossy().to_string()),
        ("PHPRC".to_string(), data_dir.to_string_lossy().to_string()),
    ];
    if icu_data_path(data_dir).is_file() {
        envs.push((
            "ICU_DATA".to_string(),
            data_dir.join("assets").to_string_lossy().to_string(),
        ));
    }
    envs
}

/// Hasilkan php-fpm.conf per server: listen loopback backend_port,
/// daemonize=no (siklus hidup dikendalikan core), pool dynamic kecil
/// (max_children=2, hemat RAM perangkat), output worker tertangkap,
/// error log per server, dan php_admin_value yang selaras php.ini.
pub fn generate_fpm_config(input: &FpmConfigInput) -> String {
    let home = fpm_home(&input.data_dir, &input.server_id);
    let pid = home.join("php-fpm.pid").to_string_lossy().to_string();
    let error_log = fpm_error_log_path(&input.data_dir, &input.server_id)
        .to_string_lossy()
        .to_string();
    let tmp = php_tmp_dir(&input.data_dir).to_string_lossy().to_string();
    let sessions = php_sessions_dir(&input.data_dir).to_string_lossy().to_string();
    let mut conf = format!(
        "; Dihasilkan otomatis oleh WebForge Mobile — jangan edit manual.\n\
         [global]\n\
         pid = {pid}\n\
         error_log = {error_log}\n\
         daemonize = no\n\
         \n\
         [www]\n\
         listen = 127.0.0.1:{port}\n\
         listen.allowed_clients = 127.0.0.1\n\
         pm = dynamic\n\
         pm.max_children = 2\n\
         pm.start_servers = 1\n\
         pm.min_spare_servers = 1\n\
         pm.max_spare_servers = 1\n\
         catch_workers_output = yes\n\
         clear_env = no\n\
         env[TMPDIR] = {tmp}\n\
         env[PHPRC] = {data_dir}\n",
        pid = pid,
        error_log = error_log,
        port = input.backend_port,
        tmp = tmp,
        data_dir = input.data_dir.to_string_lossy(),
    );
    if icu_data_path(&input.data_dir).is_file() {
        conf.push_str(&format!(
            "env[ICU_DATA] = {}\n",
            input.data_dir.join("assets").to_string_lossy()
        ));
    }
    if let Some(cmd) = sendmail_command(&input.data_dir, input.native_lib_dir.as_deref()) {
        conf.push_str(&format!("php_admin_value[sendmail_path] = {}\n", cmd));
    }
    conf.push_str(&format!(
        "php_admin_value[upload_tmp_dir] = {tmp}\n\
         php_admin_value[session.save_path] = {sessions}\n\
         php_admin_value[sys_temp_dir] = {tmp}\n",
        tmp = tmp,
        sessions = sessions,
    ));
    conf
}

/// Tulis php-fpm.conf per server (folder logs dibuat lebih dulu karena
/// validasi `php-fpm -t` membuka error_log).
pub fn write_fpm_config(input: &FpmConfigInput) -> std::io::Result<PathBuf> {
    let home = fpm_home(&input.data_dir, &input.server_id);
    std::fs::create_dir_all(home.join("logs"))?;
    std::fs::create_dir_all(php_tmp_dir(&input.data_dir))?;
    std::fs::create_dir_all(php_sessions_dir(&input.data_dir))?;
    let path = fpm_config_path(&input.data_dir, &input.server_id);
    std::fs::write(&path, generate_fpm_config(input))?;
    Ok(path)
}

/// Argumen validasi FPM: `php-fpm -t -y <conf>`.
pub fn fpm_validate_args(config_path: &Path) -> Vec<String> {
    vec![
        "-t".to_string(),
        "-y".to_string(),
        config_path.to_string_lossy().to_string(),
    ]
}

/// Argumen peluncuran FPM (foreground lewat daemonize=no di config).
pub fn fpm_launch_args(config_path: &Path) -> Vec<String> {
    vec!["-y".to_string(), config_path.to_string_lossy().to_string()]
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
    fn ini_sendmail_and_cacert_when_assets_exist() {
        let dir = std::env::temp_dir().join(format!("wf-php-ini-{}", crate::util::new_id()));
        let native = dir.join("native");
        let data = dir.join("data");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::create_dir_all(data.join("assets")).unwrap();
        std::fs::write(native.join("libsendmail.so"), b"fake").unwrap();
        std::fs::write(data.join("assets").join("cacert.pem"), b"fake").unwrap();

        let ini = generate_php_ini_full(&PhpSettings::default(), &data, Some(&native));
        assert!(ini.contains("sendmail_path"), "ini: {}", ini);
        assert!(ini.contains("libsendmail.so"), "ini: {}", ini);
        assert!(ini.contains("mail/smtp.json"), "ini: {}", ini);
        assert!(ini.contains("openssl.cafile"), "ini: {}", ini);
        assert!(ini.contains("curl.cainfo"), "ini: {}", ini);
        assert!(ini.contains("cacert.pem"), "ini: {}", ini);

        // Tanpa cacert.pem, baris CA tidak ditulis.
        std::fs::remove_file(data.join("assets").join("cacert.pem")).unwrap();
        let ini = generate_php_ini_full(&PhpSettings::default(), &data, Some(&native));
        assert!(!ini.contains("openssl.cafile"), "ini: {}", ini);
        std::fs::remove_dir_all(&dir).ok();
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
        let fpm = "PHP 8.3.37 (fpm-fcgi) (built: Jan  1 2026 00:00:00)\nCopyright (c) The PHP Group\n";
        assert_eq!(parse_php_version(fpm).as_deref(), Some("8.3.37"));
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

    #[test]
    fn find_binary_prefers_default_version_then_others() {
        let dir = std::env::temp_dir().join(format!("wf-php-ver-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("libphp84.so"), b"fake").unwrap();
        std::fs::write(dir.join("libphp82.so"), b"fake").unwrap();
        // Tanpa 8.3, default jalur lama memilih 8.4 (tertinggi).
        assert_eq!(find_php_binary(Some(&dir)), Some(dir.join("libphp84.so")));
        std::fs::write(dir.join("libphp83.so"), b"fake").unwrap();
        assert_eq!(find_php_binary(Some(&dir)), Some(dir.join("libphp83.so")));
        assert_eq!(
            find_php_binary_for_version(Some(&dir), "8.2"),
            Some(dir.join("libphp82.so"))
        );
        assert!(is_bundled_php(&dir.join("libphp83.so"), Some(&dir)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalize_and_validate_version_choice() {
        assert_eq!(normalize_php_version("8.3").as_deref(), Some("8.3"));
        assert_eq!(normalize_php_version("8.3.37").as_deref(), Some("8.3"));
        assert_eq!(normalize_php_version("php8.2").as_deref(), Some("8.2"));
        assert!(normalize_php_version("delapan").is_none());
        assert!(normalize_php_version("8").is_none());

        assert_eq!(validate_php_version_choice(None).unwrap(), None);
        assert_eq!(validate_php_version_choice(Some("")).unwrap(), None);
        assert_eq!(
            validate_php_version_choice(Some("8.4.28")).unwrap().as_deref(),
            Some("8.4")
        );
        assert!(validate_php_version_choice(Some("7.4")).is_err());
        assert!(validate_php_version_choice(Some("abc")).is_err());
    }

    fn version_info(version: &str, available: bool) -> PhpVersionInfo {
        PhpVersionInfo {
            version: version.to_string(),
            patch_version: if available { Some(format!("{}.0", version)) } else { None },
            available,
            fpm_available: false,
            extensions: Vec::new(),
            binary_path: if available { Some(format!("/bin/php{}", version)) } else { None },
            fpm_binary_path: None,
            error: if available { None } else { Some("tidak ada".to_string()) },
        }
    }

    #[test]
    fn default_version_prefers_83_else_highest() {
        let all = vec![version_info("8.2", true), version_info("8.3", true), version_info("8.4", true)];
        assert_eq!(default_php_version(&all).as_deref(), Some("8.3"));

        let no83 = vec![version_info("8.2", true), version_info("8.3", false), version_info("8.4", true)];
        assert_eq!(default_php_version(&no83).as_deref(), Some("8.4"));

        let none = vec![version_info("8.2", false), version_info("8.3", false)];
        assert!(default_php_version(&none).is_none());
    }

    #[test]
    fn php_info_from_versions_selects_requested() {
        let versions = vec![version_info("8.2", true), version_info("8.3", true)];
        let info = PhpInfo::from_versions(
            versions,
            ComposerInfo::unavailable("tidak ada".to_string()),
            Some("8.2"),
        );
        assert_eq!(info.selected_version.as_deref(), Some("8.2"));
        assert_eq!(info.version.as_deref(), Some("8.2.0"));
        assert!(info.available);
        assert_eq!(info.versions.len(), 2);

        let reselected = info.select(None);
        assert_eq!(reselected.selected_version.as_deref(), Some("8.3"));
    }

    #[test]
    fn parse_composer_version_output() {
        let out = "Composer version 2.8.4 2024-12-11 11:57:47\nPHP version 8.3.14 (/usr/bin/php)\n";
        assert_eq!(parse_composer_version(out).as_deref(), Some("2.8.4"));
        assert!(parse_composer_version("bukan composer").is_none());
    }

    #[test]
    fn fpm_config_contents() {
        let dir = std::env::temp_dir().join(format!("wf-fpm-{}", crate::util::new_id()));
        let data = dir.join("data");
        let native = dir.join("native");
        std::fs::create_dir_all(data.join("assets")).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("libsendmail.so"), b"fake").unwrap();

        let input = FpmConfigInput {
            data_dir: data.clone(),
            server_id: "srv-1".to_string(),
            backend_port: 19001,
            native_lib_dir: Some(native.clone()),
        };
        let conf = generate_fpm_config(&input);
        assert!(conf.contains("listen = 127.0.0.1:19001"), "conf: {}", conf);
        assert!(conf.contains("daemonize = no"), "conf: {}", conf);
        assert!(conf.contains("pm = dynamic"), "conf: {}", conf);
        assert!(conf.contains("pm.max_children = 2"), "conf: {}", conf);
        assert!(conf.contains("catch_workers_output = yes"), "conf: {}", conf);
        assert!(conf.contains("error_log = "), "conf: {}", conf);
        assert!(conf.contains("fpm-error.log"), "conf: {}", conf);
        assert!(conf.contains("php_admin_value[sendmail_path]"), "conf: {}", conf);
        assert!(conf.contains("php_admin_value[upload_tmp_dir]"), "conf: {}", conf);
        assert!(conf.contains("php_admin_value[session.save_path]"), "conf: {}", conf);
        assert!(conf.contains("env[TMPDIR]"), "conf: {}", conf);
        assert!(conf.contains("env[PHPRC]"), "conf: {}", conf);
        // ICU_DATA hanya bila icudt74l.dat ada.
        assert!(!conf.contains("ICU_DATA"), "conf: {}", conf);
        std::fs::write(data.join("assets").join("icudt74l.dat"), b"fake").unwrap();
        let conf = generate_fpm_config(&input);
        assert!(conf.contains("env[ICU_DATA]"), "conf: {}", conf);
        assert!(conf.contains("assets"), "conf: {}", conf);

        let path = write_fpm_config(&input).unwrap();
        assert!(path.is_file());
        assert!(fpm_home(&data, "srv-1").join("logs").is_dir());
        assert_eq!(
            fpm_validate_args(&path),
            vec!["-t".to_string(), "-y".to_string(), path.to_string_lossy().to_string()]
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
