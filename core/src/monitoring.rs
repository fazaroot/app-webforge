use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    pub port: u16,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct SystemMetrics {
    pub cpu_percent: Option<f64>,
    pub mem_total_bytes: Option<i64>,
    pub mem_used_bytes: Option<i64>,
    pub storage_total_bytes: Option<i64>,
    pub storage_used_bytes: Option<i64>,
    pub ports: Vec<PortInfo>,
    pub unavailable_reason: Option<String>,
}

fn read_cpu_times() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().next()?;
    let parts: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() < 4 {
        return None;
    }
    let idle = parts[3] + parts.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = parts.iter().sum();
    Some((total, idle))
}

pub async fn cpu_percent() -> Option<f64> {
    let (t1, i1) = read_cpu_times()?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (t2, i2) = read_cpu_times()?;
    let total_delta = t2.saturating_sub(t1) as f64;
    if total_delta == 0.0 {
        return Some(0.0);
    }
    let idle_delta = i2.saturating_sub(i1) as f64;
    Some(((total_delta - idle_delta) / total_delta * 100.0).clamp(0.0, 100.0))
}

pub fn memory_info() -> Option<(i64, i64)> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total_kb: Option<i64> = None;
    let mut avail_kb: Option<i64> = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("MemTotal:") {
            total_kb = v.split_whitespace().next().and_then(|x| x.parse().ok());
        } else if let Some(v) = line.strip_prefix("MemAvailable:") {
            avail_kb = v.split_whitespace().next().and_then(|x| x.parse().ok());
        }
    }
    let total = total_kb? * 1024;
    let avail = avail_kb? * 1024;
    Some((total, total - avail))
}

pub fn storage_info(path: &Path) -> Option<(i64, i64)> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) };
        if rc != 0 {
            return None;
        }
        let total = (stat.f_blocks as i64).checked_mul(stat.f_frsize as i64)?;
        let free = (stat.f_bfree as i64).checked_mul(stat.f_frsize as i64)?;
        Some((total, total - free))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

pub fn listening_ports() -> Vec<PortInfo> {
    let mut ports: Vec<u16> = Vec::new();
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(file) {
            for line in text.lines().skip(1) {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() < 4 {
                    continue;
                }
                if fields[3] != "0A" {
                    continue; // LISTEN
                }
                if let Some((_, port_hex)) = fields[1].rsplit_once(':') {
                    if let Ok(port) = u16::from_str_radix(port_hex, 16) {
                        if !ports.contains(&port) {
                            ports.push(port);
                        }
                    }
                }
            }
        }
    }
    ports.sort_unstable();
    ports.into_iter().map(|port| PortInfo { port, state: "LISTEN".to_string() }).collect()
}

pub async fn collect_metrics(workspace: &Path) -> SystemMetrics {
    let proc_available = Path::new("/proc/stat").exists();
    let cpu = cpu_percent().await;
    let mem = memory_info();
    let storage = storage_info(workspace);
    let ports = listening_ports();
    let unavailable_reason = if !proc_available {
        Some("/proc tidak tersedia di platform ini".to_string())
    } else if cpu.is_none() && mem.is_none() {
        Some("Gagal membaca /proc/stat dan /proc/meminfo".to_string())
    } else {
        None
    };
    SystemMetrics {
        cpu_percent: cpu,
        mem_total_bytes: mem.map(|m| m.0),
        mem_used_bytes: mem.map(|m| m.1),
        storage_total_bytes: storage.map(|s| s.0),
        storage_used_bytes: storage.map(|s| s.1),
        ports,
        unavailable_reason,
    }
}

pub fn spawn_sampler(pool: sqlx::SqlitePool, workspace: std::path::PathBuf) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let m = collect_metrics(&workspace).await;
            let _ = sqlx::query(
                "INSERT INTO monitoring_metrics (id, cpu_percent, mem_used_bytes, storage_used_bytes, recorded_at) VALUES (?1,?2,?3,?4,?5)",
            )
            .bind(crate::util::new_id())
            .bind(m.cpu_percent)
            .bind(m.mem_used_bytes)
            .bind(m.storage_used_bytes)
            .bind(crate::util::now_iso())
            .execute(&pool)
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_info_for_temp() {
        let s = storage_info(&std::env::temp_dir());
        assert!(s.is_some());
        let (total, used) = s.unwrap();
        assert!(total > 0);
        assert!(used >= 0 && used <= total);
    }

    #[tokio::test]
    async fn cpu_and_memory_on_linux() {
        if Path::new("/proc/stat").exists() {
            let c = cpu_percent().await;
            assert!(c.is_some());
            let v = c.unwrap();
            assert!((0.0..=100.0).contains(&v));
            assert!(memory_info().is_some());
        }
    }

    #[test]
    fn listening_ports_parse() {
        // On Linux /proc/net/tcp should parse without panic; result may be empty or not.
        let _ = listening_ports();
    }
}
