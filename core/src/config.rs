use rand::RngCore;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub workspace_dir: PathBuf,
    pub port: u16,
    pub token: String,
}

impl Config {
    pub fn databases_dir(&self) -> PathBuf {
        self.data_dir.join("databases")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("webforge.db")
    }

    pub fn from_env() -> anyhow_like::Result<Self> {
        let data_dir = std::env::var("WEBFORGE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./webforge-data"));
        let workspace_dir = std::env::var("WEBFORGE_WORKSPACE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("workspace"));
        let port: u16 = std::env::var("WEBFORGE_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8765);
        Self::new(data_dir, workspace_dir, port)
    }

    pub fn new(data_dir: PathBuf, workspace_dir: PathBuf, port: u16) -> anyhow_like::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(&workspace_dir)?;
        std::fs::create_dir_all(data_dir.join("databases"))?;
        std::fs::create_dir_all(data_dir.join("backups"))?;
        // canonicalize so path checks are stable
        let data_dir = data_dir.canonicalize().unwrap_or(data_dir);
        let workspace_dir = workspace_dir.canonicalize().unwrap_or(workspace_dir);
        let token = load_or_create_token(&data_dir)?;
        Ok(Self { data_dir, workspace_dir, port, token })
    }
}

fn load_or_create_token(data_dir: &PathBuf) -> anyhow_like::Result<String> {
    if let Ok(t) = std::env::var("WEBFORGE_API_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let path = data_dir.join("api.token");
    if path.exists() {
        let t = std::fs::read_to_string(&path)?;
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        use std::io::Write;
        write!(f, "{}", token)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, &token)?;
    }
    Ok(token)
}

pub mod anyhow_like {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
