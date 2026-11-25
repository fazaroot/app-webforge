use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
    pub reason: Option<String>,
    pub source: String,
}

/// Hasil probe nyata satu runtime: proses versi yang benar-benar
/// dijalankan (atau probe NodeService), lengkap dengan output mentahnya
/// agar UI bisa menampilkan bukti, bukan sekadar label "bundled".
#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub ok: bool,
    pub version: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

impl ProbeResult {
    fn failed(error: String) -> Self {
        Self {
            ok: false,
            version: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            duration_ms: 0,
            error: Some(error),
        }
    }
}

/// Konteks probe: lokasi binary bundled, data dir (untuk env spawn dan
/// kontrak probe Node), dan apakah NodeService sedang dipakai server
/// Node aktif (probe Node tidak boleh mengganggu server yang berjalan).
#[derive(Debug, Clone)]
pub struct ProbeContext {
    pub native_lib_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub node_busy: bool,
}

impl ProbeContext {
    pub fn from_globals() -> Self {
        Self {
            native_lib_dir: crate::state::native_lib_dir(),
            data_dir: crate::state::data_dir(),
            node_busy: false,
        }
    }

    pub async fn from_state(state: &crate::state::AppState) -> Self {
        let node_busy = state
            .processes
            .lock()
            .await
            .values()
            .any(|h| matches!(h, crate::state::ProcessHandle::Node(_)));
        Self {
            native_lib_dir: state.native_lib_dir.clone(),
            data_dir: Some(state.config.data_dir.clone()),
            node_busy,
        }
    }

    /// Folder basis untuk HOME/TMPDIR proses probe.
    fn home(&self) -> PathBuf {
        self.data_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("webforge-probe"))
    }
}

const REASON: &str = "Belum dibundel — lihat docs/RUNTIME_PLAN.md";
/// Timeout probe per binary (maks 3 detik).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// TTL cache probe runtime.
pub const RUNTIME_CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_PROBE_OUTPUT_CHARS: usize = 8192;

fn truncate_output(s: &str) -> String {
    if s.chars().count() <= MAX_PROBE_OUTPUT_CHARS {
        s.to_string()
    } else {
        s.chars().take(MAX_PROBE_OUTPUT_CHARS).collect()
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(120).collect::<String>())
}

pub(crate) fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(bin);
        if cand.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(m) = cand.metadata() {
                    if m.permissions().mode() & 0o111 != 0 {
                        return Some(cand);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                return Some(cand);
            }
        }
    }
    None
}

/// Binary terbundel di nativeLibraryDir (file .so hasil ekstraksi APK).
pub fn bundled_binary(native_lib_dir: Option<&Path>, file_name: &str) -> Option<PathBuf> {
    let dir = native_lib_dir?;
    let cand = dir.join(file_name);
    if cand.is_file() {
        Some(cand)
    } else {
        None
    }
}

/// Cari binary runtime: bundled .so lebih dulu, lalu nama PATH.
pub fn find_runtime_binary(
    native_lib_dir: Option<&Path>,
    bundled_name: &str,
    path_names: &[&str],
) -> Option<(PathBuf, bool)> {
    if let Some(p) = bundled_binary(native_lib_dir, bundled_name) {
        return Some((p, true));
    }
    path_names
        .iter()
        .find_map(|b| find_in_path(b))
        .map(|p| (p, false))
}

/// Jalankan perintah versi nyata dengan timeout 3 detik dan environment
/// spawn terpusat. Runtime hanya dianggap tersedia bila prosesnya
/// benar-benar berjalan dan keluar sukses.
async fn probe_binary(
    path: &Path,
    args: &[&str],
    envs: &[(String, String)],
    ctx: &ProbeContext,
) -> ProbeResult {
    let started = Instant::now();
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args);
    let home = ctx.home();
    crate::util::prepare_spawn(&mut cmd, ctx.native_lib_dir.as_deref(), &home, Some(&home));
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await;
    let duration_ms = started.elapsed().as_millis() as u64;
    match out {
        Ok(Ok(o)) => {
            let stdout = truncate_output(&String::from_utf8_lossy(&o.stdout));
            let stderr = truncate_output(&String::from_utf8_lossy(&o.stderr));
            let combined = format!("{}\n{}", stdout, stderr);
            let version = first_line(&combined);
            if o.status.success() {
                ProbeResult {
                    ok: true,
                    version,
                    stdout,
                    stderr,
                    exit_code: o.status.code(),
                    duration_ms,
                    error: None,
                }
            } else {
                ProbeResult {
                    ok: false,
                    version,
                    stdout,
                    stderr,
                    exit_code: o.status.code(),
                    duration_ms,
                    error: Some(format!(
                        "Binary {} gagal dijalankan (exit code {})",
                        path.display(),
                        o.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".to_string())
                    )),
                }
            }
        }
        Ok(Err(e)) => ProbeResult {
            ok: false,
            version: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            duration_ms,
            error: Some(format!("Binary {} tidak dapat dieksekusi: {}", path.display(), e)),
        },
        Err(_) => ProbeResult {
            ok: false,
            version: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            duration_ms,
            error: Some(format!(
                "Binary {} timeout saat probe versi ({} detik)",
                path.display(),
                PROBE_TIMEOUT.as_secs()
            )),
        },
    }
}

fn unavailable(id: &str, name: &str, reason: String) -> RuntimeInfo {
    RuntimeInfo {
        id: id.to_string(),
        name: name.to_string(),
        available: false,
        version: None,
        reason: Some(reason),
        source: "none".to_string(),
    }
}

fn info_from_probe(id: &str, name: &str, probe: &ProbeResult, bundled: bool) -> RuntimeInfo {
    RuntimeInfo {
        id: id.to_string(),
        name: name.to_string(),
        available: probe.ok,
        version: probe.version.clone(),
        reason: probe.error.clone(),
        source: if bundled { "bundled".to_string() } else { "path".to_string() },
    }
}

fn static_info() -> RuntimeInfo {
    RuntimeInfo {
        id: "static".to_string(),
        name: "WebForge Static Server".to_string(),
        available: true,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        reason: None,
        source: "bundled".to_string(),
    }
}

fn static_probe() -> (RuntimeInfo, ProbeResult) {
    let version = env!("CARGO_PKG_VERSION").to_string();
    (
        static_info(),
        ProbeResult {
            ok: true,
            version: Some(version.clone()),
            stdout: format!("WebForge Static Server (core {})", version),
            stderr: String::new(),
            exit_code: None,
            duration_ms: 0,
            error: None,
        },
    )
}

async fn generic_runtime(
    id: &str,
    name: &str,
    bundled_name: &str,
    path_names: &[&str],
    version_args: &[&str],
    ctx: &ProbeContext,
) -> (RuntimeInfo, ProbeResult) {
    match find_runtime_binary(ctx.native_lib_dir.as_deref(), bundled_name, path_names) {
        Some((path, bundled)) => {
            let probe = probe_binary(&path, version_args, &[], ctx).await;
            (info_from_probe(id, name, &probe, bundled), probe)
        }
        None => (
            unavailable(id, name, REASON.to_string()),
            ProbeResult::failed(REASON.to_string()),
        ),
    }
}

async fn php_runtime(ctx: &ProbeContext) -> (RuntimeInfo, ProbeResult) {
    match crate::php::find_php_binary(ctx.native_lib_dir.as_deref()) {
        Some(path) => {
            let mut probe = probe_binary(&path, &["-v"], &[], ctx).await;
            let parsed = crate::php::parse_php_version(&probe.stdout);
            if probe.ok && parsed.is_none() {
                let stderr = probe.stderr.trim().to_string();
                probe.ok = false;
                probe.error = Some(if stderr.is_empty() {
                    "php -v gagal atau output tidak dikenal".to_string()
                } else {
                    format!(
                        "php -v gagal atau output tidak dikenal: {}",
                        stderr.chars().take(300).collect::<String>()
                    )
                });
            }
            probe.version = parsed;
            let bundled = crate::php::is_bundled_php(&path, ctx.native_lib_dir.as_deref());
            (info_from_probe("php", "PHP", &probe, bundled), probe)
        }
        None => {
            let reason = "Binary PHP tidak ditemukan: libphp.so tidak ada di nativeLibraryDir dan php tidak ada di PATH".to_string();
            (unavailable("php", "PHP", reason.clone()), ProbeResult::failed(reason))
        }
    }
}

async fn python_runtime(ctx: &ProbeContext) -> (RuntimeInfo, ProbeResult) {
    match crate::python_runtime::find_python_binary(ctx.native_lib_dir.as_deref()) {
        Some(path) => {
            let bundled = crate::python_runtime::is_bundled_python(&path, ctx.native_lib_dir.as_deref());
            let mut envs = Vec::new();
            if let Some(data_dir) = &ctx.data_dir {
                if crate::python_runtime::stdlib_dir_path(data_dir).is_dir() {
                    envs.push((
                        "PYTHONHOME".to_string(),
                        crate::python_runtime::python_home(data_dir)
                            .to_string_lossy()
                            .to_string(),
                    ));
                }
            }
            let mut probe = probe_binary(&path, &["--version"], &envs, ctx).await;
            probe.version = probe
                .version
                .map(|v| v.trim_start_matches("Python ").to_string());
            (info_from_probe("python", "Python", &probe, bundled), probe)
        }
        None => (
            unavailable("python", "Python", REASON.to_string()),
            ProbeResult::failed(REASON.to_string()),
        ),
    }
}

/// Probe Node lewat NodeService (:node) dengan kontrak file di
/// `<data_dir>/node-probe`. Hanya Android: NodeService dijalankan lewat
/// JNI yang sama dengan jalur server Node.
#[cfg(target_os = "android")]
async fn probe_node_service(ctx: &ProbeContext) -> ProbeResult {
    let started = Instant::now();
    if ctx.node_busy {
        // NodeService hanya bisa menjalankan satu Node::Start per proses
        // :node. Server Node aktif sudah melewati health check, jadi
        // runtime terbukti bekerja; probe dilewati agar server tidak
        // diganggu.
        return ProbeResult {
            ok: true,
            version: None,
            stdout: "NodeService sedang menjalankan server Node aktif; probe dilewati agar server tidak terganggu".to_string(),
            stderr: String::new(),
            exit_code: None,
            duration_ms: 0,
            error: None,
        };
    }
    let Some(data_dir) = ctx.data_dir.clone() else {
        return ProbeResult::failed(
            "data_dir core tidak diketahui untuk probe Node".to_string(),
        );
    };
    let dir = crate::node_runtime::node_probe_dir(&data_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return ProbeResult::failed(format!("Gagal membuat folder probe Node: {}", e));
    }
    let nonce = crate::util::new_id();
    let script_path = crate::node_runtime::node_probe_script_path(&data_dir);
    if let Err(e) = std::fs::write(&script_path, crate::node_runtime::node_probe_script()) {
        return ProbeResult::failed(format!("Gagal menulis skrip probe Node: {}", e));
    }
    let request = serde_json::json!({"nonce": nonce}).to_string();
    if let Err(e) = std::fs::write(dir.join("request.json"), request) {
        return ProbeResult::failed(format!("Gagal menulis request probe Node: {}", e));
    }
    let result_path = dir.join("result.json");
    let _ = std::fs::remove_file(&result_path);
    let log_path = dir.join("node-probe.log");
    let _ = std::fs::remove_file(&log_path);
    let status_path = dir.join("node-probe-status.txt");
    let args = crate::node_runtime::node_probe_args(&script_path);
    let started_ok = crate::jni_bridge::node_start(
        &args,
        &dir.to_string_lossy(),
        &log_path.to_string_lossy(),
        &status_path.to_string_lossy(),
    );
    if !started_ok {
        return ProbeResult {
            ok: false,
            version: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            duration_ms: started.elapsed().as_millis() as u64,
            error: Some("NodeService gagal dimulai lewat JNI untuk probe Node".to_string()),
        };
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let mut last_error: Option<String> = None;
    loop {
        if let Ok(text) = std::fs::read_to_string(&result_path) {
            match crate::node_runtime::parse_node_probe_result(&text, &nonce) {
                Ok(result) => {
                    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                    let status = std::fs::read_to_string(&status_path).unwrap_or_default();
                    let exit_code = status.trim().parse::<i32>().ok();
                    let duration_ms = started.elapsed().as_millis() as u64;
                    if let Some(err) = result.error.filter(|e| !e.trim().is_empty()) {
                        return ProbeResult {
                            ok: false,
                            version: None,
                            stdout: truncate_output(&log),
                            stderr: String::new(),
                            exit_code,
                            duration_ms,
                            error: Some(format!("Probe Node gagal: {}", err)),
                        };
                    }
                    return match result.version.filter(|v| !v.trim().is_empty()) {
                        Some(version) => ProbeResult {
                            ok: true,
                            version: Some(version),
                            stdout: truncate_output(&log),
                            stderr: String::new(),
                            exit_code,
                            duration_ms,
                            error: None,
                        },
                        None => ProbeResult {
                            ok: false,
                            version: None,
                            stdout: truncate_output(&log),
                            stderr: String::new(),
                            exit_code,
                            duration_ms,
                            error: Some("Hasil probe Node tidak berisi versi".to_string()),
                        },
                    };
                }
                Err(e) => last_error = Some(e),
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = crate::jni_bridge::node_stop();
            let status = std::fs::read_to_string(&status_path).unwrap_or_default();
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            let log_tail: Vec<&str> = log.lines().rev().take(10).collect();
            let log_tail: Vec<&str> = log_tail.into_iter().rev().collect();
            return ProbeResult {
                ok: false,
                version: None,
                stdout: truncate_output(&log),
                stderr: String::new(),
                exit_code: status.trim().parse::<i32>().ok(),
                duration_ms: started.elapsed().as_millis() as u64,
                error: Some(format!(
                    "Probe Node timeout setelah 6 detik: {} (status NodeService: {}; log: {})",
                    last_error.unwrap_or_else(|| "result.json tidak muncul".to_string()),
                    status.trim(),
                    log_tail.join("\n")
                )),
            };
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn node_runtime(ctx: &ProbeContext) -> (RuntimeInfo, ProbeResult) {
    let libnode = bundled_binary(ctx.native_lib_dir.as_deref(), "libnode.so");
    let shim = bundled_binary(ctx.native_lib_dir.as_deref(), "libwebforge_node.so");
    #[cfg(target_os = "android")]
    {
        if libnode.is_some() && shim.is_some() {
            let probe = probe_node_service(ctx).await;
            return (info_from_probe("node", "Node.js", &probe, true), probe);
        }
    }
    if let Some(path) = find_in_path("node") {
        let probe = probe_binary(&path, &["--version"], &[], ctx).await;
        return (info_from_probe("node", "Node.js", &probe, false), probe);
    }
    if libnode.is_some() != shim.is_some() {
        let reason = format!(
            "Bundle Node tidak lengkap: libnode.so {}, libwebforge_node.so {}",
            if libnode.is_some() { "ada" } else { "tidak ada" },
            if shim.is_some() { "ada" } else { "tidak ada" }
        );
        return (unavailable("node", "Node.js", reason.clone()), ProbeResult::failed(reason));
    }
    (
        unavailable("node", "Node.js", REASON.to_string()),
        ProbeResult::failed(REASON.to_string()),
    )
}

async fn postgres_runtime(ctx: &ProbeContext) -> (RuntimeInfo, ProbeResult) {
    let bundled = bundled_binary(ctx.native_lib_dir.as_deref(), "libpostgres.so");
    let initdb = bundled_binary(ctx.native_lib_dir.as_deref(), "libinitdb.so");
    let pgctl = bundled_binary(ctx.native_lib_dir.as_deref(), "libpgctl.so");
    let found = match (bundled, initdb, pgctl) {
        (Some(path), Some(_), Some(_)) => Some((path, true)),
        _ => find_in_path("postgres").map(|p| (p, false)),
    };
    match found {
        Some((path, is_bundled)) => {
            let mut probe = probe_binary(&path, &["--version"], &[], ctx).await;
            probe.version = probe
                .version
                .and_then(|v| crate::db_runtime::parse_postgres_version(&v));
            (info_from_probe("postgresql", "PostgreSQL", &probe, is_bundled), probe)
        }
        None => (
            unavailable("postgresql", "PostgreSQL", REASON.to_string()),
            ProbeResult::failed(REASON.to_string()),
        ),
    }
}

async fn mariadb_runtime(ctx: &ProbeContext) -> (RuntimeInfo, ProbeResult) {
    let found = bundled_binary(ctx.native_lib_dir.as_deref(), "libmariadbd.so")
        .map(|p| (p, true))
        .or_else(|| find_in_path("mariadbd").or_else(|| find_in_path("mysqld")).map(|p| (p, false)));
    match found {
        Some((path, is_bundled)) => {
            let mut probe = probe_binary(&path, &["--version"], &[], ctx).await;
            probe.version = probe
                .version
                .and_then(|v| crate::db_runtime::parse_mariadb_version(&v));
            (info_from_probe("mariadb", "MariaDB", &probe, is_bundled), probe)
        }
        None => (
            unavailable("mariadb", "MariaDB", REASON.to_string()),
            ProbeResult::failed(REASON.to_string()),
        ),
    }
}

/// Probe segar satu runtime berdasarkan id. `None` bila id tidak dikenal.
pub async fn probe_runtime_by_id(id: &str, ctx: &ProbeContext) -> Option<(RuntimeInfo, ProbeResult)> {
    match id {
        "static" => Some(static_probe()),
        "php" => Some(php_runtime(ctx).await),
        "caddy" => Some(generic_runtime("caddy", "Caddy", "libcaddy.so", &["caddy"], &["version"], ctx).await),
        "nginx" => Some(generic_runtime("nginx", "Nginx", "libnginx.so", &["nginx"], &["-v"], ctx).await),
        "apache" => Some(generic_runtime("apache", "Apache", "libhttpd.so", &["httpd", "apache2"], &["-v"], ctx).await),
        "python" => Some(python_runtime(ctx).await),
        "node" => Some(node_runtime(ctx).await),
        "mariadb" => Some(mariadb_runtime(ctx).await),
        "postgresql" => Some(postgres_runtime(ctx).await),
        _ => None,
    }
}

/// Probe semua runtime secara paralel. Urutan hasil tetap:
/// static, php, caddy, nginx, apache, python, node, mariadb, postgresql.
pub async fn detect_runtimes_ctx(ctx: &ProbeContext) -> Vec<RuntimeInfo> {
    let (php, caddy, nginx, apache, python, node, mariadb, postgresql) = tokio::join!(
        php_runtime(ctx),
        generic_runtime("caddy", "Caddy", "libcaddy.so", &["caddy"], &["version"], ctx),
        generic_runtime("nginx", "Nginx", "libnginx.so", &["nginx"], &["-v"], ctx),
        generic_runtime("apache", "Apache", "libhttpd.so", &["httpd", "apache2"], &["-v"], ctx),
        python_runtime(ctx),
        node_runtime(ctx),
        mariadb_runtime(ctx),
        postgres_runtime(ctx),
    );
    vec![
        static_info(),
        php.0,
        caddy.0,
        nginx.0,
        apache.0,
        python.0,
        node.0,
        mariadb.0,
        postgresql.0,
    ]
}

pub async fn detect_runtimes() -> Vec<RuntimeInfo> {
    detect_runtimes_ctx(&ProbeContext::from_globals()).await
}

pub async fn detect_runtimes_with(native_lib_dir: Option<PathBuf>) -> Vec<RuntimeInfo> {
    let ctx = ProbeContext {
        native_lib_dir,
        data_dir: crate::state::data_dir(),
        node_busy: false,
    };
    detect_runtimes_ctx(&ctx).await
}

/// Cache hasil probe runtime di AppState. Probe hanya dijalankan sekali
/// per TTL (60 detik) atau saat dipaksa (`fresh`); pemanggil bersamaan
/// diserialkan oleh lock agar tidak ada badai probe.
pub struct RuntimeCache {
    inner: tokio::sync::Mutex<Option<(Instant, Vec<RuntimeInfo>)>>,
    ttl: Duration,
}

impl RuntimeCache {
    pub fn new() -> Self {
        Self::with_ttl(RUNTIME_CACHE_TTL)
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(None),
            ttl,
        }
    }

    pub async fn get_or_probe<F, Fut>(&self, fresh: bool, probe: F) -> Vec<RuntimeInfo>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Vec<RuntimeInfo>>,
    {
        let mut guard = self.inner.lock().await;
        if !fresh {
            if let Some((at, runtimes)) = guard.as_ref() {
                if at.elapsed() < self.ttl {
                    return runtimes.clone();
                }
            }
        }
        let runtimes = probe().await;
        *guard = Some((Instant::now(), runtimes.clone()));
        runtimes
    }

    /// Perbarui satu entri cache dari probe segar satu runtime (hasil
    /// endpoint test), tanpa mengubah timestamp TTL keseluruhan.
    pub async fn update_entry(&self, info: RuntimeInfo) {
        let mut guard = self.inner.lock().await;
        if let Some((_, runtimes)) = guard.as_mut() {
            if let Some(slot) = runtimes.iter_mut().find(|r| r.id == info.id) {
                *slot = info;
            }
        }
    }
}

impl Default for RuntimeCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Hasil probe runtime dari cache AppState; `fresh` memaksa probe ulang.
pub async fn runtimes_cached(state: &crate::state::AppState, fresh: bool) -> Vec<RuntimeInfo> {
    let ctx = ProbeContext::from_state(state).await;
    state
        .runtime_cache
        .get_or_probe(fresh, || detect_runtimes_ctx(&ctx))
        .await
}

/// Probe segar satu runtime untuk endpoint test; entri cache runtime
/// itu diperbarui dengan hasil segar.
pub async fn test_runtime(
    state: &crate::state::AppState,
    id: &str,
) -> Option<(RuntimeInfo, ProbeResult)> {
    let ctx = ProbeContext::from_state(state).await;
    let (info, probe) = probe_runtime_by_id(id, &ctx).await?;
    state.runtime_cache.update_entry(info.clone()).await;
    Some((info, probe))
}

pub async fn available_runtime_ids(state: &crate::state::AppState) -> HashSet<String> {
    runtimes_cached(state, false)
        .await
        .into_iter()
        .filter(|r| r.available)
        .map(|r| r.id)
        .collect()
}

pub async fn sync_runtime_table(state: &crate::state::AppState, fresh: bool) -> Vec<RuntimeInfo> {
    let runtimes = runtimes_cached(state, fresh).await;
    for r in &runtimes {
        let _ = sqlx::query(
            "INSERT INTO runtime_versions (id, name, available, version, reason, source, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, available=excluded.available, version=excluded.version, reason=excluded.reason, source=excluded.source, updated_at=excluded.updated_at",
        )
        .bind(&r.id)
        .bind(&r.name)
        .bind(if r.available { 1 } else { 0 })
        .bind(&r.version)
        .bind(&r.reason)
        .bind(&r.source)
        .bind(crate::util::now_iso())
        .execute(&state.pool)
        .await;
    }
    runtimes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn find_in_path_missing() {
        assert!(find_in_path("webforge-definitely-not-a-binary-xyz").is_none());
    }

    #[tokio::test]
    async fn static_always_available_and_all_runtimes_listed() {
        let r = detect_runtimes().await;
        assert_eq!(r.len(), 9);
        assert!(r[0].available);
        assert_eq!(r[0].id, "static");
        let ids: Vec<&str> = r.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["static", "php", "caddy", "nginx", "apache", "python", "node", "mariadb", "postgresql"]
        );
        for other in &r[1..] {
            if !other.available {
                // Runtime tidak tersedia wajib punya alasan jujur. Source
                // boleh "path"/"bundled" bila binary ditemukan tetapi probe
                // versinya gagal di host uji.
                assert!(other.reason.is_some(), "runtime {} tidak tersedia tanpa alasan", other.id);
            }
        }
    }

    #[tokio::test]
    async fn cache_does_not_reprobe_within_ttl() {
        let cache = RuntimeCache::with_ttl(Duration::from_secs(60));
        let counter = Arc::new(AtomicUsize::new(0));

        let c = counter.clone();
        cache
            .get_or_probe(false, move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                vec![static_info()]
            })
            .await;
        let c = counter.clone();
        cache
            .get_or_probe(false, move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                vec![static_info()]
            })
            .await;
        assert_eq!(counter.load(Ordering::SeqCst), 1, "probe kedua harus dari cache");

        let c = counter.clone();
        cache
            .get_or_probe(true, move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                vec![static_info()]
            })
            .await;
        assert_eq!(counter.load(Ordering::SeqCst), 2, "fresh harus mem-probe ulang");
    }

    #[tokio::test]
    async fn cache_reprobes_after_ttl_expires() {
        let cache = RuntimeCache::with_ttl(Duration::ZERO);
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let c = counter.clone();
            cache
                .get_or_probe(false, move || async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    vec![static_info()]
                })
                .await;
        }
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn static_probe_reports_core_version() {
        let ctx = ProbeContext::from_globals();
        let (info, probe) = probe_runtime_by_id("static", &ctx).await.unwrap();
        assert!(info.available);
        assert!(probe.ok);
        assert_eq!(probe.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert!(probe.stdout.contains(env!("CARGO_PKG_VERSION")));
        assert!(probe.error.is_none());
        assert!(probe_runtime_by_id("tidak-ada", &ctx).await.is_none());
    }
}
