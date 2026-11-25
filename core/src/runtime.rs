use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
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
const REASON_PHP: &str =
    "Binary PHP tidak ditemukan: libphp.so tidak ada di nativeLibraryDir dan php tidak ada di PATH";

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

async fn binary_version(path: &PathBuf) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new(path).arg("--version").output(),
    )
    .await
    .ok()?
    .ok()?;
    let text = if !out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        String::from_utf8_lossy(&out.stderr).to_string()
    };
    let first = text.lines().next()?.trim().to_string();
    if first.is_empty() {
        None
    } else {
        Some(first.chars().take(100).collect())
    }
}

async fn php_runtime_info(native_lib_dir: Option<&std::path::Path>) -> RuntimeInfo {
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
        None => RuntimeInfo {
            id: "php".to_string(),
            name: "PHP".to_string(),
            available: false,
            version: None,
            reason: Some(REASON_PHP.to_string()),
            source: "none".to_string(),
        },
    }
}

pub async fn detect_runtimes() -> Vec<RuntimeInfo> {
    detect_runtimes_with(crate::state::native_lib_dir()).await
}

pub async fn detect_runtimes_with(native_lib_dir: Option<PathBuf>) -> Vec<RuntimeInfo> {
    let mut out = vec![RuntimeInfo {
        id: "static".to_string(),
        name: "WebForge Static Server".to_string(),
        available: true,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        reason: None,
        source: "bundled".to_string(),
    }];
    out.push(php_runtime_info(native_lib_dir.as_deref()).await);
    let defs: [(&str, &str, &[&str]); 6] = [
        ("nginx", "Nginx", &["nginx"]),
        ("apache", "Apache", &["apache2", "httpd"]),
        ("python", "Python", &["python3", "python"]),
        ("node", "Node.js", &["node"]),
        ("mariadb", "MariaDB", &["mariadbd", "mysqld"]),
        ("postgresql", "PostgreSQL", &["postgres"]),
    ];
    for (id, name, bins) in defs {
        let found = bins.iter().find_map(|b| find_in_path(b));
        match found {
            Some(p) => {
                let version = binary_version(&p).await;
                out.push(RuntimeInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                    available: true,
                    version,
                    reason: None,
                    source: "path".to_string(),
                });
            }
            None => out.push(RuntimeInfo {
                id: id.to_string(),
                name: name.to_string(),
                available: false,
                version: None,
                reason: Some(REASON.to_string()),
                source: "none".to_string(),
            }),
        }
    }
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
    async fn static_always_available() {
        let r = detect_runtimes().await;
        assert_eq!(r.len(), 8);
        assert!(r[0].available);
        assert_eq!(r[0].id, "static");
        for other in &r[1..] {
            if !other.available {
                assert!(other.reason.is_some());
                assert_eq!(other.source, "none");
            }
        }
    }
}
