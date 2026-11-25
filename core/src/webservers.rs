//! Generator konfigurasi web server eksternal (Caddy, Nginx, Apache httpd)
//! untuk WebForge Mobile v0.3.0.
//!
//! Modul ini hanya menghasilkan teks konfigurasi dan argumen proses; ia
//! tidak men-spawn proses apa pun. Koordinator (process.rs) yang bertugas:
//!
//! 1. menemukan binary bundled (`libcaddy.so`, `libnginx.so`,
//!    `libhttpd.so`) di nativeLibraryDir,
//! 2. memanggil [`write_config`] lebih dulu (membuat folder log/tmp yang
//!    dibutuhkan saat validasi),
//! 3. menjalankan validasi dengan [`validate_args`], dan hanya meluncurkan
//!    proses bila validasi lolos,
//! 4. meluncurkan proses dengan [`launch_args`] serta environment yang
//!    mengarah ke folder data aplikasi (`HOME`, `TMPDIR`, dan untuk Caddy
//!    `XDG_DATA_HOME`/`XDG_CONFIG_HOME` ke server home), karena Android
//!    tidak menyediakan `/tmp`/`$HOME` yang bisa ditulis aplikasi.
//!
//! Keputusan teknis yang disengaja:
//!
//! - Caddy: `admin off` — reload konfigurasi dilakukan dengan restart
//!   proses oleh core, bukan lewat admin API. Storage diarahkan eksplisit
//!   ke server home agar Caddy tidak menulis ke `$HOME`.
//! - Nginx: `daemon off` + `master_process off` — satu proses tunggal,
//!   sehingga kill langsung ke child oleh core benar-benar menghentikan
//!   server dan membebaskan port (master/worker akan meninggalkan worker
//!   yatim yang tetap memegang port).
//! - Mode proxy (Caddy/Nginx): semua request diteruskan ke backend
//!   (`reverse_proxy`/`proxy_pass`), file statis tidak dilayani langsung
//!   oleh web server. Backend (mis. `php -S` atau server Node) yang
//!   bertanggung jawab atas file statisnya sendiri. Ini menghindari
//!   routing ganda yang membingungkan (sebagian path dari disk, sebagian
//!   dari backend).
//! - Apache httpd: reverse proxy TIDAK didukung pada build v0.3.0
//!   (mod_proxy tidak dikompilasi statis; lihat
//!   scripts/native/build-apache.sh). [`generate_httpd_config`]
//!   mengembalikan `Err` jujur bila `proxy_port` diminta, bukan config
//!   yang diam-diam mengabaikan proxy. Pengguna yang butuh proxy Apache
//!   bisa menyediakan `custom_config` sendiri.
//! - Apache httpd berjalan dengan MPM yang men-spawn child process.
//!   Core saat ini membunuh child langsung (SIGKILL ke proses induk);
//!   koordinator harus memastikan child httpd ikut mati (mis. kill
//!   process group atau SIGTERM ke induk lebih dulu), bila tidak port
//!   bisa tetap terpegang child yatim. Ini dicatat sebagai risiko
//!   integrasi, bukan disembunyikan.

use std::path::{Path, PathBuf};

/// Masukan untuk menghasilkan konfigurasi satu instance web server.
///
/// `project_path` adalah folder proyek (induk `doc_root`); generator saat
/// ini hanya memakai `doc_root` sebagai document root, tetapi field ini
/// dipertahankan agar koordinator bisa memakai path proyek (mis. untuk
/// cwd proses atau validasi) tanpa mengubah struktur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebServerInput {
    pub host: String,
    pub port: u16,
    pub doc_root: PathBuf,
    pub project_path: PathBuf,
    pub data_dir: PathBuf,
    pub server_id: String,
    pub proxy_port: Option<u16>,
    pub custom_config: Option<String>,
}

/// Folder home satu instance server: `<data_dir>/servers/<id>/<engine>`.
///
/// `server_id` dan `engine` adalah identifier internal (UUID database dan
/// konstanta engine), bukan masukan pengguna. Pemisah path tetap diganti
/// `_` secara defensif agar nilai tak terduga tidak bisa keluar dari
/// folder `servers/`.
pub fn server_home(data_dir: &Path, server_id: &str, engine: &str) -> PathBuf {
    let safe_id = server_id.replace(['/', '\\'], "_");
    let safe_engine = engine.replace(['/', '\\'], "_");
    data_dir.join("servers").join(safe_id).join(safe_engine)
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Format `host:port`, dengan kurung siku untuk literal IPv6.
fn format_host_port(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

/// Token path Caddyfile: tanpa kutip bila aman, berkutip ganda bila
/// mengandung spasi atau karakter khusus.
fn caddy_path(path: &Path) -> String {
    let raw = path_str(path);
    if raw.chars().any(|c| c.is_whitespace() || c == '"' || c == '#') {
        format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        raw
    }
}

/// Token path Nginx: tanpa kutip bila aman, berkutip ganda bila
/// mengandung karakter yang bermakna dalam sintaks Nginx.
fn nginx_path(path: &Path) -> String {
    let raw = path_str(path);
    let needs_quote = raw
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, ';' | '{' | '}' | '#' | '$' | '"' | '\'' | '\\'));
    if needs_quote {
        format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        raw
    }
}

/// Path Apache selalu berkutip ganda (gaya httpd.conf baku).
fn apache_quoted(path: &Path) -> String {
    format!(
        "\"{}\"",
        path_str(path).replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Hasilkan Caddyfile untuk satu instance.
///
/// Site address memakai skema `http://` eksplisit agar Caddy tidak
/// mencoba TLS otomatis (ACME/internal CA) untuk alamat lokal. Log
/// diarahkan ke stderr agar tertangkap log per-server core.
pub fn generate_caddyfile(input: &WebServerInput) -> String {
    let home = server_home(&input.data_dir, &input.server_id, "caddy");
    let addr = format_host_port(&input.host, input.port);
    let root = caddy_path(&input.doc_root);
    let storage = caddy_path(&home.join("caddy-data"));
    let (handler, handler_comment) = match input.proxy_port {
        Some(proxy_port) => (
            format!("reverse_proxy 127.0.0.1:{}", proxy_port),
            "# Mode proxy: semua request diteruskan ke backend; file statis\n    # dilayani backend, bukan langsung dari disk oleh Caddy.\n    ",
        ),
        None => ("file_server".to_string(), ""),
    };
    format!(
        "# Dihasilkan otomatis oleh WebForge Mobile — jangan edit manual.\n\
         # Admin API dimatikan: perubahan config diterapkan dengan restart proses.\n\
         {{\n\
         \x20   admin off\n\
         \x20   auto_https off\n\
         \x20   storage file_system {{\n\
         \x20       root {storage}\n\
         \x20   }}\n\
         \x20   log {{\n\
         \x20       output stderr\n\
         \x20   }}\n\
         }}\n\
         \n\
         http://{addr} {{\n\
         \x20   root * {root}\n\
         \x20   {handler_comment}{handler}\n\
         \x20   log {{\n\
         \x20       output stderr\n\
         \x20   }}\n\
         }}\n",
        storage = storage,
        addr = addr,
        root = root,
        handler_comment = handler_comment,
        handler = handler,
    )
}

/// Tabel MIME bersama: (tipe, ekstensi). Dipakai untuk mime.types Nginx
/// (format `types { ... }`) dan mime.types Apache (format baris polos).
const MIME_TABLE: &[(&str, &[&str])] = &[
    ("text/html", &["html", "htm", "shtml"]),
    ("text/css", &["css"]),
    ("text/javascript", &["js", "mjs"]),
    ("application/json", &["json", "map"]),
    ("application/manifest+json", &["webmanifest"]),
    ("text/plain", &["txt"]),
    ("text/markdown", &["md"]),
    ("text/csv", &["csv"]),
    ("text/xml", &["xml"]),
    ("application/xhtml+xml", &["xhtml"]),
    ("application/rss+xml", &["rss"]),
    ("application/atom+xml", &["atom"]),
    ("application/pdf", &["pdf"]),
    ("application/wasm", &["wasm"]),
    ("application/zip", &["zip"]),
    ("application/gzip", &["gz"]),
    ("application/x-tar", &["tar"]),
    ("application/octet-stream", &["bin", "deb", "dmg", "iso", "img", "msi"]),
    ("image/png", &["png"]),
    ("image/jpeg", &["jpg", "jpeg"]),
    ("image/gif", &["gif"]),
    ("image/webp", &["webp"]),
    ("image/avif", &["avif"]),
    ("image/svg+xml", &["svg", "svgz"]),
    ("image/x-icon", &["ico"]),
    ("image/bmp", &["bmp"]),
    ("image/tiff", &["tif", "tiff"]),
    ("font/woff", &["woff"]),
    ("font/woff2", &["woff2"]),
    ("font/ttf", &["ttf"]),
    ("font/otf", &["otf"]),
    ("application/vnd.ms-fontobject", &["eot"]),
    ("audio/mpeg", &["mp3"]),
    ("audio/ogg", &["ogg"]),
    ("video/mp4", &["mp4"]),
    ("video/webm", &["webm"]),
    ("video/ogg", &["ogv"]),
];

/// Hasilkan isi `mime.types` format Nginx (`types { ... }`).
pub fn generate_mime_types() -> String {
    let mut out = String::from("# Dihasilkan otomatis oleh WebForge Mobile.\ntypes {\n");
    for (mime, exts) in MIME_TABLE {
        out.push_str(&format!("    {} {};\n", mime, exts.join(" ")));
    }
    out.push_str("}\n");
    out
}

/// Isi `mime.types` format Apache (baris `tipe ekstensi...` tanpa
/// kurung/semicolon), dipakai `TypesConfig` httpd. Tanpa file ini httpd
/// gagal start karena TypesConfig bawaan (`conf/mime.types` relatif
/// ServerRoot) tidak ada di server home.
fn apache_mime_types() -> String {
    let mut out = String::from("# Dihasilkan otomatis oleh WebForge Mobile.\n");
    for (mime, exts) in MIME_TABLE {
        out.push_str(&format!("{} {}\n", mime, exts.join(" ")));
    }
    out
}

/// Hasilkan konfigurasi Nginx untuk satu instance.
///
/// `include mime.types;` bersifat relatif terhadap prefix (`-p` server
/// home), dan [`write_config`] menulis file itu di server home. Semua
/// path runtime (pid, error log, temp path) berada di server home karena
/// prefix compile-time tidak ada di Android. Access log dimatikan agar
/// file log tidak tumbuh tanpa batas di HP; error log tetap ditulis ke
/// file di server home.
pub fn generate_nginx_config(input: &WebServerInput) -> String {
    let home = server_home(&input.data_dir, &input.server_id, "nginx");
    let addr = format_host_port(&input.host, input.port);
    let pid = nginx_path(&home.join("nginx.pid"));
    let error_log = nginx_path(&home.join("logs").join("error.log"));
    let root = nginx_path(&input.doc_root);
    let tmp = home.join("tmp");
    let client_body_tmp = nginx_path(&tmp.join("client_body"));
    let proxy_tmp = nginx_path(&tmp.join("proxy"));
    let fastcgi_tmp = nginx_path(&tmp.join("fastcgi"));
    let uwsgi_tmp = nginx_path(&tmp.join("uwsgi"));
    let scgi_tmp = nginx_path(&tmp.join("scgi"));

    let location = match input.proxy_port {
        Some(proxy_port) => format!(
            "# Mode proxy: semua request diteruskan ke backend; file statis\n\
             \x20           # dilayani backend, bukan langsung dari disk oleh Nginx.\n\
             \x20           location / {{\n\
             \x20               proxy_pass http://127.0.0.1:{proxy_port};\n\
             \x20               proxy_set_header Host $host;\n\
             \x20               proxy_set_header X-Real-IP $remote_addr;\n\
             \x20               proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
             \x20               proxy_set_header X-Forwarded-Proto $scheme;\n\
             \x20           }}",
            proxy_port = proxy_port,
        ),
        None => "location / {\n\
                 \x20               try_files $uri $uri/ =404;\n\
                 \x20           }"
            .to_string(),
    };

    format!(
        "# Dihasilkan otomatis oleh WebForge Mobile — jangan edit manual.\n\
         # master_process off: satu proses tunggal agar penghentian oleh core\n\
         # benar-benar membebaskan port (tanpa worker yatim).\n\
         daemon off;\n\
         master_process off;\n\
         pid {pid};\n\
         error_log {error_log};\n\
         worker_processes 1;\n\
         \n\
         events {{\n\
         \x20   worker_connections 256;\n\
         }}\n\
         \n\
         http {{\n\
         \x20   include mime.types;\n\
         \x20   default_type application/octet-stream;\n\
         \x20   access_log off;\n\
         \x20   sendfile on;\n\
         \x20   client_max_body_size 64m;\n\
         \x20   client_body_temp_path {client_body_tmp};\n\
         \x20   proxy_temp_path {proxy_tmp};\n\
         \x20   fastcgi_temp_path {fastcgi_tmp};\n\
         \x20   uwsgi_temp_path {uwsgi_tmp};\n\
         \x20   scgi_temp_path {scgi_tmp};\n\
         \n\
         \x20   server {{\n\
         \x20       listen {addr};\n\
         \x20       server_name _;\n\
         \x20       root {root};\n\
         \x20       index index.html index.htm;\n\
         \n\
         \x20       {location}\n\
         \x20   }}\n\
         }}\n",
        pid = pid,
        error_log = error_log,
        client_body_tmp = client_body_tmp,
        proxy_tmp = proxy_tmp,
        fastcgi_tmp = fastcgi_tmp,
        uwsgi_tmp = uwsgi_tmp,
        scgi_tmp = scgi_tmp,
        addr = addr,
        root = root,
        location = location,
    )
}

/// Hasilkan konfigurasi Apache httpd untuk satu instance.
///
/// Mengembalikan `Err` bila `proxy_port` diminta: build httpd WebForge
/// v0.3.0 tidak mengompilasi mod_proxy (lihat
/// scripts/native/build-apache.sh), jadi reverse proxy Apache tidak
/// didukung dan tidak akan dipalsukan dengan config yang mengabaikan
/// proxy. Modul yang dipakai config ini (mime, dir, log_config,
/// authz_core, authz_host, authn_core, authz_user, unixd, setenvif,
/// alias, rewrite, env, headers, reqtimeout) dikompilasi statis ke
/// binary; karena itu tidak ada baris `LoadModule`.
///
/// `AllowOverride All` pada document root disengaja agar `.htaccess`
/// (nilai khas Apache, mis. WordPress) berfungsi. Mutex memakai
/// mekanisme file `flock` di server home karena Android tidak
/// menyediakan SysV semaphore/POSIX named semaphore yang diasumsikan
/// mekanisme bawaan APR di Linux desktop.
pub fn generate_httpd_config(input: &WebServerInput) -> Result<String, String> {
    if let Some(proxy_port) = input.proxy_port {
        return Err(format!(
            "Apache httpd WebForge v0.3.0 tidak mendukung reverse proxy (proxy_port {} diminta): mod_proxy tidak dikompilasi statis pada build ini. Gunakan Caddy atau Nginx untuk reverse proxy, atau sediakan custom_config Apache sendiri.",
            proxy_port
        ));
    }
    let home = server_home(&input.data_dir, &input.server_id, "apache");
    let addr = format_host_port(&input.host, input.port);
    let server_root = apache_quoted(&home);
    let pid_file = apache_quoted(&home.join("httpd.pid"));
    let error_log = apache_quoted(&home.join("logs").join("error.log"));
    let access_log = apache_quoted(&home.join("logs").join("access.log"));
    let types_config = apache_quoted(&home.join("mime.types"));
    let runtime_dir = apache_quoted(&home.join("logs"));
    let doc_root = apache_quoted(&input.doc_root);
    let logs_dir = path_str(&home.join("logs"));
    let mutex = if logs_dir.chars().any(|c| c.is_whitespace()) {
        format!("\"flock:{}\"", logs_dir.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("flock:{}", logs_dir)
    };

    Ok(format!(
        "# Dihasilkan otomatis oleh WebForge Mobile — jangan edit manual.\n\
         # Modul dikompilasi statis ke binary (tanpa LoadModule): mime, dir,\n\
         # log_config, authz_core, authz_host, authn_core, authz_user, unixd,\n\
         # setenvif, alias, rewrite, env, headers, reqtimeout, ssl,\n\
         # socache_shmcb, watchdog.\n\
         ServerRoot {server_root}\n\
         ServerName localhost\n\
         Listen {addr}\n\
         PidFile {pid_file}\n\
         ErrorLog {error_log}\n\
         LogLevel warn\n\
         CustomLog {access_log} common\n\
         TypesConfig {types_config}\n\
         DefaultRuntimeDir {runtime_dir}\n\
         Mutex {mutex}\n\
         ServerTokens Prod\n\
         ServerSignature Off\n\
         TraceEnable Off\n\
         Timeout 60\n\
         KeepAlive On\n\
         \n\
         <IfModule mpm_event_module>\n\
         \x20   StartServers 1\n\
         \x20   MinSpareThreads 5\n\
         \x20   MaxSpareThreads 10\n\
         \x20   ThreadsPerChild 25\n\
         \x20   MaxRequestWorkers 25\n\
         \x20   MaxConnectionsPerChild 0\n\
         </IfModule>\n\
         <IfModule mpm_prefork_module>\n\
         \x20   StartServers 1\n\
         \x20   MinSpareServers 1\n\
         \x20   MaxSpareServers 3\n\
         \x20   MaxRequestWorkers 10\n\
         \x20   MaxConnectionsPerChild 0\n\
         </IfModule>\n\
         \n\
         <Directory />\n\
         \x20   AllowOverride none\n\
         \x20   Require all denied\n\
         </Directory>\n\
         <Directory {doc_root}>\n\
         \x20   Options FollowSymLinks\n\
         \x20   AllowOverride All\n\
         \x20   Require all granted\n\
         </Directory>\n\
         <Files \".ht*\">\n\
         \x20   Require all denied\n\
         </Files>\n\
         \n\
         <VirtualHost *:{port}>\n\
         \x20   ServerName localhost\n\
         \x20   DocumentRoot {doc_root}\n\
         \x20   DirectoryIndex index.html index.htm\n\
         </VirtualHost>\n",
        server_root = server_root,
        addr = addr,
        pid_file = pid_file,
        error_log = error_log,
        access_log = access_log,
        types_config = types_config,
        runtime_dir = runtime_dir,
        mutex = mutex,
        doc_root = doc_root,
        port = input.port,
    ))
}

/// Nama file konfigurasi per engine di server home. `None` untuk engine
/// yang tidak didukung modul ini. Engine kanonis: `caddy`, `nginx`,
/// `apache` (`httpd` diterima sebagai alias Apache).
pub fn config_filename(engine: &str) -> Option<&'static str> {
    match engine {
        "caddy" => Some("Caddyfile"),
        "nginx" => Some("nginx.conf"),
        "apache" | "httpd" => Some("httpd.conf"),
        _ => None,
    }
}

/// Tulis konfigurasi satu instance ke server home dan kembalikan path
/// file konfigurasi yang ditulis.
///
/// `custom_config` yang tidak kosong menang atas hasil generate (ditulis
/// apa adanya; validasi isi tetap tanggung jawab langkah validasi
/// koordinator). Untuk Nginx ditulis juga `mime.types` format Nginx,
/// untuk Apache `mime.types` format Apache yang dirujuk `TypesConfig`.
/// Folder log/tmp dibuat di sini karena validasi (`nginx -t`,
/// `httpd -t`, `caddy validate`) membuka file log/storage di server
/// home; panggil fungsi ini sebelum validasi.
///
/// Catatan: struktur [`WebServerInput`] tidak membawa field engine,
/// jadi engine diteruskan sebagai parameter terpisah.
pub fn write_config(input: &WebServerInput, engine: &str) -> std::io::Result<PathBuf> {
    let filename = config_filename(engine).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Engine web server tidak didukung: {}", engine),
        )
    })?;
    let home = server_home(&input.data_dir, &input.server_id, engine);
    std::fs::create_dir_all(home.join("logs"))?;
    std::fs::create_dir_all(home.join("tmp"))?;
    match engine {
        "caddy" => {
            std::fs::create_dir_all(home.join("caddy-data"))?;
        }
        "nginx" => {
            for sub in ["client_body", "proxy", "fastcgi", "uwsgi", "scgi"] {
                std::fs::create_dir_all(home.join("tmp").join(sub))?;
            }
        }
        _ => {}
    }

    let custom = input
        .custom_config
        .as_deref()
        .filter(|text| !text.trim().is_empty());
    let content = match custom {
        Some(text) => text.to_string(),
        None => match engine {
            "caddy" => generate_caddyfile(input),
            "nginx" => generate_nginx_config(input),
            "apache" | "httpd" => generate_httpd_config(input).map_err(|message| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
            })?,
            _ => unreachable!("engine sudah divalidasi config_filename"),
        },
    };

    let config_path = home.join(filename);
    std::fs::write(&config_path, content)?;
    match engine {
        "nginx" => {
            std::fs::write(home.join("mime.types"), generate_mime_types())?;
        }
        "apache" | "httpd" => {
            std::fs::write(home.join("mime.types"), apache_mime_types())?;
        }
        _ => {}
    }
    Ok(config_path)
}

/// Argumen validasi konfigurasi per engine. Vektor kosong berarti engine
/// tidak didukung; pemanggil wajib memeriksa kekosongan itu dan tidak
/// meluncurkan proses.
pub fn validate_args(engine: &str, config_path: &Path, home: &Path) -> Vec<String> {
    match engine {
        "caddy" => vec![
            "validate".to_string(),
            "--config".to_string(),
            path_str(config_path),
            "--adapter".to_string(),
            "caddyfile".to_string(),
        ],
        "nginx" => vec![
            "-t".to_string(),
            "-c".to_string(),
            path_str(config_path),
            "-p".to_string(),
            path_str(home),
            "-g".to_string(),
            "daemon off;".to_string(),
        ],
        "apache" | "httpd" => vec![
            "-t".to_string(),
            "-f".to_string(),
            path_str(config_path),
            "-d".to_string(),
            path_str(home),
        ],
        _ => Vec::new(),
    }
}

/// Argumen peluncuran proses per engine (mode foreground semua, agar
/// siklus hidup proses dikendalikan core). Vektor kosong berarti engine
/// tidak didukung.
pub fn launch_args(engine: &str, config_path: &Path, home: &Path) -> Vec<String> {
    match engine {
        "caddy" => vec![
            "run".to_string(),
            "--config".to_string(),
            path_str(config_path),
            "--adapter".to_string(),
            "caddyfile".to_string(),
        ],
        "nginx" => vec![
            "-c".to_string(),
            path_str(config_path),
            "-p".to_string(),
            path_str(home),
            "-g".to_string(),
            "daemon off;".to_string(),
        ],
        "apache" | "httpd" => vec![
            "-f".to_string(),
            path_str(config_path),
            "-d".to_string(),
            path_str(home),
            "-DFOREGROUND".to_string(),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(proxy_port: Option<u16>) -> WebServerInput {
        WebServerInput {
            host: "127.0.0.1".to_string(),
            port: 8080,
            doc_root: PathBuf::from("/data/workspace/site/public"),
            project_path: PathBuf::from("/data/workspace/site"),
            data_dir: PathBuf::from("/data/webforge"),
            server_id: "srv-1".to_string(),
            proxy_port,
            custom_config: None,
        }
    }

    fn temp_data_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "wf-webservers-{}-{}-{}",
            name,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn server_home_layout() {
        assert_eq!(
            server_home(Path::new("/data/webforge"), "srv-1", "nginx"),
            PathBuf::from("/data/webforge/servers/srv-1/nginx")
        );
    }

    #[test]
    fn caddyfile_static_has_bind_root_and_file_server() {
        let text = generate_caddyfile(&sample_input(None));
        assert!(text.contains("admin off"), "caddyfile: {}", text);
        assert!(text.contains("http://127.0.0.1:8080"), "caddyfile: {}", text);
        assert!(
            text.contains("root * /data/workspace/site/public"),
            "caddyfile: {}",
            text
        );
        assert!(text.contains("file_server"), "caddyfile: {}", text);
        assert!(text.contains("output stderr"), "caddyfile: {}", text);
        assert!(!text.contains("reverse_proxy"), "caddyfile: {}", text);
    }

    #[test]
    fn caddyfile_proxy_has_reverse_proxy() {
        let text = generate_caddyfile(&sample_input(Some(9000)));
        assert!(text.contains("http://127.0.0.1:8080"), "caddyfile: {}", text);
        assert!(
            text.contains("root * /data/workspace/site/public"),
            "caddyfile: {}",
            text
        );
        assert!(
            text.contains("reverse_proxy 127.0.0.1:9000"),
            "caddyfile: {}",
            text
        );
        assert!(!text.contains("file_server"), "caddyfile: {}", text);
    }

    #[test]
    fn nginx_static_has_listen_root_and_try_files() {
        let text = generate_nginx_config(&sample_input(None));
        assert!(text.contains("daemon off;"), "nginx: {}", text);
        assert!(text.contains("master_process off;"), "nginx: {}", text);
        assert!(
            text.contains("pid /data/webforge/servers/srv-1/nginx/nginx.pid;"),
            "nginx: {}",
            text
        );
        assert!(
            text.contains("error_log /data/webforge/servers/srv-1/nginx/logs/error.log;"),
            "nginx: {}",
            text
        );
        assert!(text.contains("include mime.types;"), "nginx: {}", text);
        assert!(text.contains("listen 127.0.0.1:8080;"), "nginx: {}", text);
        assert!(
            text.contains("root /data/workspace/site/public;"),
            "nginx: {}",
            text
        );
        assert!(text.contains("try_files $uri $uri/ =404;"), "nginx: {}", text);
        assert!(!text.contains("proxy_pass"), "nginx: {}", text);
    }

    #[test]
    fn nginx_proxy_has_proxy_pass_and_headers() {
        let text = generate_nginx_config(&sample_input(Some(9000)));
        assert!(text.contains("listen 127.0.0.1:8080;"), "nginx: {}", text);
        assert!(
            text.contains("root /data/workspace/site/public;"),
            "nginx: {}",
            text
        );
        assert!(
            text.contains("proxy_pass http://127.0.0.1:9000;"),
            "nginx: {}",
            text
        );
        assert!(
            text.contains("proxy_set_header Host $host;"),
            "nginx: {}",
            text
        );
        assert!(
            text.contains("proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"),
            "nginx: {}",
            text
        );
        assert!(!text.contains("try_files"), "nginx: {}", text);
    }

    #[test]
    fn mime_types_has_common_entries() {
        let text = generate_mime_types();
        assert!(text.contains("types {"), "mime: {}", text);
        assert!(text.contains("text/html html htm shtml;"), "mime: {}", text);
        assert!(text.contains("text/css css;"), "mime: {}", text);
        assert!(text.contains("image/png png;"), "mime: {}", text);
        assert!(text.contains("application/wasm wasm;"), "mime: {}", text);
    }

    #[test]
    fn httpd_static_has_listen_and_document_root() {
        let text = generate_httpd_config(&sample_input(None)).unwrap();
        assert!(text.contains("Listen 127.0.0.1:8080"), "httpd: {}", text);
        assert!(
            text.contains("DocumentRoot \"/data/workspace/site/public\""),
            "httpd: {}",
            text
        );
        assert!(
            text.contains("ServerRoot \"/data/webforge/servers/srv-1/apache\""),
            "httpd: {}",
            text
        );
        assert!(text.contains("DirectoryIndex index.html"), "httpd: {}", text);
        assert!(
            !text
                .lines()
                .any(|line| line.trim_start().starts_with("LoadModule")),
            "httpd: {}",
            text
        );
    }

    #[test]
    fn httpd_proxy_is_honest_error() {
        let err = generate_httpd_config(&sample_input(Some(9000))).unwrap_err();
        assert!(err.contains("tidak mendukung reverse proxy"), "err: {}", err);
        assert!(err.contains("9000"), "err: {}", err);
    }

    #[test]
    fn filenames_and_args_exact() {
        assert_eq!(config_filename("caddy"), Some("Caddyfile"));
        assert_eq!(config_filename("nginx"), Some("nginx.conf"));
        assert_eq!(config_filename("apache"), Some("httpd.conf"));
        assert_eq!(config_filename("httpd"), Some("httpd.conf"));
        assert_eq!(config_filename("lighttpd"), None);

        let cfg = Path::new("/data/webforge/servers/srv-1/nginx/nginx.conf");
        let home = Path::new("/data/webforge/servers/srv-1/nginx");
        assert_eq!(
            validate_args("nginx", cfg, home),
            vec![
                "-t".to_string(),
                "-c".to_string(),
                "/data/webforge/servers/srv-1/nginx/nginx.conf".to_string(),
                "-p".to_string(),
                "/data/webforge/servers/srv-1/nginx".to_string(),
                "-g".to_string(),
                "daemon off;".to_string(),
            ]
        );
        assert_eq!(
            launch_args("nginx", cfg, home),
            vec![
                "-c".to_string(),
                "/data/webforge/servers/srv-1/nginx/nginx.conf".to_string(),
                "-p".to_string(),
                "/data/webforge/servers/srv-1/nginx".to_string(),
                "-g".to_string(),
                "daemon off;".to_string(),
            ]
        );

        let cfg = Path::new("/data/webforge/servers/srv-1/caddy/Caddyfile");
        let home = Path::new("/data/webforge/servers/srv-1/caddy");
        assert_eq!(
            validate_args("caddy", cfg, home),
            vec![
                "validate".to_string(),
                "--config".to_string(),
                "/data/webforge/servers/srv-1/caddy/Caddyfile".to_string(),
                "--adapter".to_string(),
                "caddyfile".to_string(),
            ]
        );
        assert_eq!(
            launch_args("caddy", cfg, home),
            vec![
                "run".to_string(),
                "--config".to_string(),
                "/data/webforge/servers/srv-1/caddy/Caddyfile".to_string(),
                "--adapter".to_string(),
                "caddyfile".to_string(),
            ]
        );

        let cfg = Path::new("/data/webforge/servers/srv-1/apache/httpd.conf");
        let home = Path::new("/data/webforge/servers/srv-1/apache");
        assert_eq!(
            validate_args("apache", cfg, home),
            vec![
                "-t".to_string(),
                "-f".to_string(),
                "/data/webforge/servers/srv-1/apache/httpd.conf".to_string(),
                "-d".to_string(),
                "/data/webforge/servers/srv-1/apache".to_string(),
            ]
        );
        assert_eq!(
            launch_args("apache", cfg, home),
            vec![
                "-f".to_string(),
                "/data/webforge/servers/srv-1/apache/httpd.conf".to_string(),
                "-d".to_string(),
                "/data/webforge/servers/srv-1/apache".to_string(),
                "-DFOREGROUND".to_string(),
            ]
        );

        assert!(validate_args("lighttpd", cfg, home).is_empty());
        assert!(launch_args("lighttpd", cfg, home).is_empty());
    }

    #[test]
    fn write_config_custom_wins_over_generated() {
        let data_dir = temp_data_dir("custom");
        let mut input = sample_input(None);
        input.data_dir = data_dir.clone();
        input.custom_config = Some("# config custom pengguna\n".to_string());
        let path = write_config(&input, "nginx").unwrap();
        assert_eq!(
            path,
            data_dir.join("servers").join("srv-1").join("nginx").join("nginx.conf")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# config custom pengguna\n"
        );
        let mime = std::fs::read_to_string(
            data_dir.join("servers").join("srv-1").join("nginx").join("mime.types"),
        )
        .unwrap();
        assert!(mime.contains("types {"), "mime: {}", mime);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[test]
    fn write_config_generated_caddy_and_httpd_proxy_error() {
        let data_dir = temp_data_dir("generated");
        let mut input = sample_input(None);
        input.data_dir = data_dir.clone();
        let path = write_config(&input, "caddy").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("file_server"), "caddyfile: {}", text);
        assert!(
            data_dir
                .join("servers")
                .join("srv-1")
                .join("caddy")
                .join("caddy-data")
                .is_dir()
        );

        let mut proxied = sample_input(Some(9000));
        proxied.data_dir = data_dir.clone();
        let err = write_config(&proxied, "apache").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("tidak mendukung reverse proxy"),
            "err: {}",
            err
        );
        std::fs::remove_dir_all(&data_dir).ok();
    }
}
