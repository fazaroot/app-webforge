use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
    pub reason: Option<String>,
    pub source: String,
}

const REASON: &str = "Belum dibundel — lihat docs/RUNTIME_PLAN.md";

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

struct ProbeResult {
    ok: bool,
    version: Option<String>,
    error: Option<String>,
}

/// Jalankan perintah versi nyata dengan timeout. Runtime hanya dianggap
/// tersedia bila prosesnya benar-benar berjalan dan keluar sukses.
async fn probe_binary(path: &Path, args: &[&str], envs: &[(String, String)]) -> ProbeResult {
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await;
    match out {
        Ok(Ok(o)) => {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            let first = text
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
                .map(|l| l.chars().take(120).collect::<String>());
            if o.status.success() {
                ProbeResult { ok: true, version: first, error: None }
            } else {
                ProbeResult {
                    ok: false,
                    version: first,
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
            error: Some(format!("Binary {} tidak dapat dieksekusi: {}", path.display(), e)),
        },
        Err(_) => ProbeResult {
            ok: false,
            version: None,
            error: Some(format!("Binary {} timeout saat probe versi", path.display())),
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

async fn generic_runtime(
    id: &str,
    name: &str,
    bundled_name: &str,
    path_names: &[&str],
    version_args: &[&str],
    native_lib_dir: Option<&Path>,
) -> RuntimeInfo {
    match find_runtime_binary(native_lib_dir, bundled_name, path_names) {
        Some((path, bundled)) => {
            let probe = probe_binary(&path, version_args, &[]).await;
            RuntimeInfo {
                id: id.to_string(),
                name: name.to_string(),
                available: probe.ok,
                version: probe.version,
                reason: probe.error,
                source: if bundled { "bundled".to_string() } else { "path".to_string() },
            }
        }
        None => unavailable(id, name, REASON.to_string()),
    }
}

async fn php_runtime_info(native_lib_dir: Option<&Path>) -> RuntimeInfo {
    match crate::php::find_php_binary(native_lib_dir) {
        Some(path) => {
            let info = crate::php::probe_php_info(Some(path.clone())).await;
            RuntimeInfo {
                id: "php".to_string(),
                name: "PHP".to_string(),
                available: info.available,
                version: info.version,
                reason: info.error,
                source: if crate::php::is_bundled_php(&path, native_lib_dir) {
                    "bundled".to_string()
                } else {
                    "path".to_string()
                },
            }
        }
        None => unavailable(
            "php",
            "PHP",
            "Binary PHP tidak ditemukan: libphp.so tidak ada di nativeLibraryDir dan php tidak ada di PATH".to_string(),
        ),
    }
}

async fn python_runtime_info(native_lib_dir: Option<&Path>) -> RuntimeInfo {
    match crate::python_runtime::find_python_binary(native_lib_dir) {
        Some(path) => {
            let bundled = crate::python_runtime::is_bundled_python(&path, native_lib_dir);
            let mut envs = Vec::new();
            if let Some(data_dir) = crate::state::data_dir() {
                let home = crate::python_runtime::python_home(&data_dir);
                if crate::python_runtime::stdlib_dir_path(&data_dir).is_dir() {
                    envs.push(("PYTHONHOME".to_string(), home.to_string_lossy().to_string()));
                }
                envs.push(("HOME".to_string(), data_dir.to_string_lossy().to_string()));
                envs.push(("TMPDIR".to_string(), data_dir.join("tmp").to_string_lossy().to_string()));
            }
            let probe = probe_binary(&path, &["--version"], &envs).await;
            RuntimeInfo {
                id: "python".to_string(),
                name: "Python".to_string(),
                available: probe.ok,
                version: probe.version.map(|v| v.trim_start_matches("Python ").to_string()),
                reason: probe.error,
                source: if bundled { "bundled".to_string() } else { "path".to_string() },
            }
        }
        None => unavailable("python", "Python", REASON.to_string()),
    }
}

fn node_runtime_info(native_lib_dir: Option<&Path>) -> RuntimeInfo {
    // Node Android berjalan lewat NodeService (:node) dengan libnode.so +
    // shim libwebforge_node.so. Keduanya harus ada; versi dibaca dari file
    // versi yang disalin Kotlin dari asset hasil build CI.
    let libnode = bundled_binary(native_lib_dir, "libnode.so");
    let shim = bundled_binary(native_lib_dir, "libwebforge_node.so");
    if libnode.is_some() && shim.is_some() {
        let version = crate::state::data_dir()
            .and_then(|d| std::fs::read_to_string(d.join("assets").join("node-version.txt")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        return RuntimeInfo {
            id: "node".to_string(),
            name: "Node.js".to_string(),
            available: true,
            version,
            reason: None,
            source: "bundled".to_string(),
        };
    }
    if find_in_path("node").is_some() {
        return RuntimeInfo {
            id: "node".to_string(),
            name: "Node.js".to_string(),
            available: true,
            version: None,
            reason: None,
            source: "path".to_string(),
        };
    }
    unavailable("node", "Node.js", REASON.to_string())
}

async fn postgres_runtime_info(native_lib_dir: Option<&Path>) -> RuntimeInfo {
    let bundled = bundled_binary(native_lib_dir, "libpostgres.so");
    let initdb = bundled_binary(native_lib_dir, "libinitdb.so");
    let pgctl = bundled_binary(native_lib_dir, "libpgctl.so");
    if let (Some(path), Some(_), Some(_)) = (bundled, initdb, pgctl) {
        let probe = probe_binary(&path, &["--version"], &[]).await;
        return RuntimeInfo {
            id: "postgresql".to_string(),
            name: "PostgreSQL".to_string(),
            available: probe.ok,
            version: probe.version.and_then(|v| crate::db_runtime::parse_postgres_version(&v)),
            reason: probe.error,
            source: "bundled".to_string(),
        };
    }
    if let Some(path) = find_in_path("postgres") {
        let probe = probe_binary(&path, &["--version"], &[]).await;
        return RuntimeInfo {
            id: "postgresql".to_string(),
            name: "PostgreSQL".to_string(),
            available: probe.ok,
            version: probe.version.and_then(|v| crate::db_runtime::parse_postgres_version(&v)),
            reason: probe.error,
            source: "path".to_string(),
        };
    }
    unavailable("postgresql", "PostgreSQL", REASON.to_string())
}

async fn mariadb_runtime_info(native_lib_dir: Option<&Path>) -> RuntimeInfo {
    if let Some(path) = bundled_binary(native_lib_dir, "libmariadbd.so") {
        let probe = probe_binary(&path, &["--version"], &[]).await;
        return RuntimeInfo {
            id: "mariadb".to_string(),
            name: "MariaDB".to_string(),
            available: probe.ok,
            version: probe.version.and_then(|v| crate::db_runtime::parse_mariadb_version(&v)),
            reason: probe.error,
            source: "bundled".to_string(),
        };
    }
    if let Some(path) = find_in_path("mariadbd").or_else(|| find_in_path("mysqld")) {
        let probe = probe_binary(&path, &["--version"], &[]).await;
        return RuntimeInfo {
            id: "mariadb".to_string(),
            name: "MariaDB".to_string(),
            available: probe.ok,
            version: probe.version.and_then(|v| crate::db_runtime::parse_mariadb_version(&v)),
            reason: probe.error,
            source: "path".to_string(),
        };
    }
    unavailable("mariadb", "MariaDB", REASON.to_string())
}

pub async fn detect_runtimes() -> Vec<RuntimeInfo> {
    detect_runtimes_with(crate::state::native_lib_dir()).await
}

pub async fn detect_runtimes_with(native_lib_dir: Option<PathBuf>) -> Vec<RuntimeInfo> {
    let dir = native_lib_dir.as_deref();
    let mut out = vec![RuntimeInfo {
        id: "static".to_string(),
        name: "WebForge Static Server".to_string(),
        available: true,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        reason: None,
        source: "bundled".to_string(),
    }];
    out.push(php_runtime_info(dir).await);
    out.push(generic_runtime("caddy", "Caddy", "libcaddy.so", &["caddy"], &["version"], dir).await);
    out.push(generic_runtime("nginx", "Nginx", "libnginx.so", &["nginx"], &["-v"], dir).await);
    out.push(generic_runtime("apache", "Apache", "libhttpd.so", &["httpd", "apache2"], &["-v"], dir).await);
    out.push(python_runtime_info(dir).await);
    out.push(node_runtime_info(dir));
    out.push(mariadb_runtime_info(dir).await);
    out.push(postgres_runtime_info(dir).await);
    out
}

pub async fn available_runtime_ids() -> HashSet<String> {
    detect_runtimes()
        .await
        .into_iter()
        .filter(|r| r.available)
        .map(|r| r.id)
        .collect()
}

pub async fn sync_runtime_table(pool: &sqlx::SqlitePool) -> Vec<RuntimeInfo> {
    let runtimes = detect_runtimes().await;
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
        .execute(pool)
        .await;
    }
    runtimes
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
