//! Runtime database server lokal: PostgreSQL 16 & MariaDB 10.11.
//!
//! Modul ini hanya berisi logika murni (path, config, argumen proses,
//! parsing versi, validasi identifier) agar bisa diuji tanpa perangkat.
//! Eksekusi proses nyata dilakukan koordinator lewat `process.rs`.
//!
//! ## Layout di perangkat
//!
//! ```text
//! <data_dir>/databases-services/<instance_id>/postgres/
//!   bin/postgres   -> symlink ke <nativeLibraryDir>/libpostgres.so
//!   bin/initdb     -> symlink ke <nativeLibraryDir>/libinitdb.so
//!   bin/pg_ctl     -> symlink ke <nativeLibraryDir>/libpgctl.so
//!   bin/psql       -> symlink ke <nativeLibraryDir>/libpsql.so
//!   share/postgresql/   (hasil ekstrak postgres-share.zip)
//!   data/               (cluster PostgreSQL, dibuat initdb)
//!   run/                (unix socket PostgreSQL)
//! <data_dir>/databases-services/<instance_id>/mariadb/
//!   data/               (datadir MariaDB)
//!   run/                (socket + pid-file MariaDB)
//!   tmp/                (tmpdir MariaDB)
//!   share/mysql/        (hasil ekstrak mariadb-share.zip)
//!   my.cnf              (hasil generate_mariadb_cnf)
//!   error.log           (log-error MariaDB; dibaca log center core)
//! ```
//!
//! ## Catatan Android yang menentukan desain ini
//!
//! - Binary server dikemas sebagai `lib*.so` di nativeLibraryDir (W^X:
//!   hanya lokasi itu yang boleh dieksekusi). PostgreSQL mencari folder
//!   `share/` dan program pendamping relatif terhadap path
//!   executable-nya, jadi core WAJIB menjalankan PostgreSQL lewat
//!   symlink `bin/postgres` di home instance (argv[0] = path symlink
//!   absolut) dan mengekstrak share ke `share/postgresql`. Build
//!   PostgreSQL WebForge mempatch `normalize_exec_path` agar path
//!   argv[0] tidak di-`realpath` menembus symlink ke nativeLibraryDir;
//!   tanpa patch itu initdb mencari program "postgres" di folder
//!   `libinitdb.so` dan gagal di perangkat.
//! - Android tidak menyediakan System V shared memory/semaphore yang bisa
//!   diandalkan (kernel Android umumnya tanpa CONFIG_SYSVIPC) dan Bionic
//!   tidak menyediakan `shm_open`. Karena itu config PostgreSQL memaksa
//!   `shared_memory_type=mmap` dan `dynamic_shared_memory_type=mmap`, dan
//!   build memakai semaphore POSIX unnamed (lihat
//!   scripts/native/build-postgres.sh).
//! - `mariadb-install-db` adalah skrip shell, bukan binary ELF, jadi tidak
//!   bisa dikemas sebagai `.so`. Jalur utama inisialisasi MariaDB di
//!   Android adalah `mariadbd --bootstrap` dengan file SQL sistem dari
//!   share (lihat `mariadb_bootstrap_args` +
//!   `mariadb_bootstrap_sql_files`); `mariadb_install_args` tetap
//!   disediakan untuk lingkungan yang punya shell + coreutils lengkap.
//! - Kedua server hanya mendengarkan 127.0.0.1. Password superuser
//!   digenerate core, disimpan terenkripsi oleh koordinator, dan tidak
//!   pernah ditulis ke file config hasil modul ini.

use std::path::{Path, PathBuf};

/// Port bawaan bila koordinator tidak mengalokasikan port lain.
/// Koordinator bebas memakai port lain per instance.
pub const POSTGRES_DEFAULT_PORT: u16 = 5432;
pub const MARIADB_DEFAULT_PORT: u16 = 3306;

// ---------------------------------------------------------------------------
// Path home per instance
// ---------------------------------------------------------------------------

/// Home instance PostgreSQL: `<data_dir>/databases-services/<id>/postgres`.
///
/// `instance_id` harus sudah divalidasi koordinator (mis. UUID hasil
/// `new_id()`); fungsi ini tidak menyaring separator path.
pub fn postgres_home(data_dir: &Path, instance_id: &str) -> PathBuf {
    data_dir
        .join("databases-services")
        .join(instance_id)
        .join("postgres")
}

/// Home instance MariaDB: `<data_dir>/databases-services/<id>/mariadb`.
pub fn mariadb_home(data_dir: &Path, instance_id: &str) -> PathBuf {
    data_dir
        .join("databases-services")
        .join(instance_id)
        .join("mariadb")
}

// ---------------------------------------------------------------------------
// PostgreSQL: path turunan
// ---------------------------------------------------------------------------

/// Direktori cluster (`-D`) PostgreSQL. Dibuat oleh initdb, bukan core.
pub fn postgres_data_dir(home: &Path) -> PathBuf {
    home.join("data")
}

/// Direktori unix socket PostgreSQL (`unix_socket_directories`).
/// Harus dibuat core sebelum server start; postmaster tidak membuatnya.
pub fn postgres_run_dir(home: &Path) -> PathBuf {
    home.join("run")
}

/// Direktori share PostgreSQL hasil ekstrak postgres-share.zip
/// (berisi timezone/, postgres.bki, extension/*.control, dst).
pub fn postgres_share_dir(home: &Path) -> PathBuf {
    home.join("share").join("postgresql")
}

/// File config utama di dalam cluster. Core menimpa file bawaan initdb
/// dengan hasil `generate_postgresql_conf` setelah initdb selesai.
pub fn postgres_conf_path(home: &Path) -> PathBuf {
    postgres_data_dir(home).join("postgresql.conf")
}

/// Path symlink binary yang WAJIB dipakai sebagai argv[0] saat menjalankan
/// PostgreSQL (lihat catatan modul). Target symlink ada di
/// `postgres_symlinks`.
pub fn postgres_binary_path(home: &Path) -> PathBuf {
    home.join("bin").join("postgres")
}

pub fn postgres_initdb_path(home: &Path) -> PathBuf {
    home.join("bin").join("initdb")
}

pub fn postgres_pg_ctl_path(home: &Path) -> PathBuf {
    home.join("bin").join("pg_ctl")
}

pub fn postgres_psql_path(home: &Path) -> PathBuf {
    home.join("bin").join("psql")
}

/// Pasangan (symlink di home, target di nativeLibraryDir) yang harus
/// dibuat koordinator sebelum initdb/start pertama. Gunakan
/// [`ensure_postgres_symlinks`] untuk membuat/memperbarui symlink itu.
pub fn postgres_symlinks(home: &Path, native_lib_dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    vec![
        (
            postgres_binary_path(home),
            native_lib_dir.join("libpostgres.so"),
        ),
        (
            postgres_initdb_path(home),
            native_lib_dir.join("libinitdb.so"),
        ),
        (
            postgres_pg_ctl_path(home),
            native_lib_dir.join("libpgctl.so"),
        ),
        (
            postgres_psql_path(home),
            native_lib_dir.join("libpsql.so"),
        ),
    ]
}

/// Buat atau perbarui symlink binary PostgreSQL di home instance.
///
/// nativeLibraryDir berubah antar instalasi/update APK, jadi symlink
/// lama bisa menunjuk path yang sudah tidak ada. Fungsi ini mengganti
/// symlink yang targetnya berbeda (termasuk symlink rusak) dan menolak
/// menimpa file/direktori biasa yang kebetulan berada di path symlink.
#[cfg(unix)]
pub fn ensure_postgres_symlinks(home: &Path, native_lib_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(home.join("bin")).map_err(|e| e.to_string())?;
    for (link, target) in postgres_symlinks(home, native_lib_dir) {
        match std::fs::symlink_metadata(&link) {
            Ok(meta) if meta.file_type().is_symlink() => {
                match std::fs::read_link(&link) {
                    Ok(existing) if existing == target => continue,
                    _ => {
                        std::fs::remove_file(&link).map_err(|e| {
                            format!("Gagal mengganti symlink {}: {}", link.display(), e)
                        })?;
                    }
                }
            }
            Ok(_) => {
                return Err(format!(
                    "Path binary PostgreSQL sudah ada dan bukan symlink: {}",
                    link.display()
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "Gagal memeriksa symlink {}: {}",
                    link.display(),
                    e
                ));
            }
        }
        std::os::unix::fs::symlink(&target, &link).map_err(|e| {
            format!(
                "Gagal membuat symlink {} -> {}: {}",
                link.display(),
                target.display(),
                e
            )
        })?;
    }
    Ok(())
}

/// Di non-Unix tidak ada jalur symlink Android; host memakai binary PATH.
#[cfg(not(unix))]
pub fn ensure_postgres_symlinks(_home: &Path, _native_lib_dir: &Path) -> Result<(), String> {
    Ok(())
}

/// Direktori yang harus dibuat core sebelum initdb/start PostgreSQL.
/// `data/` sengaja tidak termasuk: initdb membuatnya sendiri dan menolak
/// direktori yang sudah berisi file lain.
pub fn postgres_required_dirs(home: &Path) -> Vec<PathBuf> {
    vec![home.to_path_buf(), postgres_run_dir(home), home.join("bin")]
}

// ---------------------------------------------------------------------------
// PostgreSQL: config & argumen
// ---------------------------------------------------------------------------

/// Kutip nilai string untuk postgresql.conf (tanda kutip tunggal digandakan).
fn pg_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Isi postgresql.conf untuk instance lokal di Android.
///
/// Hanya 127.0.0.1, shared memory mmap (Android tanpa SysV shm/shm_open),
/// buffer kecil untuk ponsel, log ke stderr agar ditangkap log center core
/// lewat pipe proses.
pub fn generate_postgresql_conf(port: u16, home: &Path) -> String {
    let run_dir = postgres_run_dir(home);
    format!(
        "# Dihasilkan WebForge Mobile core (db_runtime.rs). Jangan edit manual;\n\
         # file ini ditimpa core setiap instance disiapkan.\n\
         listen_addresses = '127.0.0.1'\n\
         port = {port}\n\
         unix_socket_directories = {run_dir}\n\
         unix_socket_permissions = 0700\n\
         shared_memory_type = mmap\n\
         dynamic_shared_memory_type = mmap\n\
         shared_buffers = '32MB'\n\
         max_connections = 20\n\
         logging_collector = off\n\
         log_destination = 'stderr'\n\
         log_min_messages = warning\n\
         timezone = 'UTC'\n\
         log_timezone = 'UTC'\n\
         datestyle = 'iso, mdy'\n\
         lc_messages = 'C'\n\
         lc_monetary = 'C'\n\
         lc_numeric = 'C'\n\
         lc_time = 'C'\n\
         default_text_search_config = 'pg_catalog.simple'\n\
         password_encryption = 'scram-sha-256'\n\
         max_wal_size = '256MB'\n\
         min_wal_size = '32MB'\n",
        port = port,
        run_dir = pg_quote(&run_dir.to_string_lossy()),
    )
}

/// Argumen initdb (tanpa argv[0]).
///
/// Keamanan: koneksi lokal lewat unix socket memakai `trust` (socket
/// berada di direktori privat aplikasi dengan izin 0700), koneksi TCP
/// 127.0.0.1 wajib `scram-sha-256`. Password superuser dibaca initdb dari
/// `pwfile` (baris pertama file); koordinator menulis pwfile dengan izin
/// 0600, memakainya sekali, lalu menghapusnya. Locale dipaksa `C` +
/// encoding UTF8 karena dukungan locale Bionic sangat terbatas dan build
/// ini tanpa ICU.
pub fn postgres_init_args(
    home: &Path,
    share_dir: &Path,
    username: &str,
    pwfile: &Path,
) -> Vec<String> {
    vec![
        "-D".to_string(),
        postgres_data_dir(home).to_string_lossy().to_string(),
        "-L".to_string(),
        share_dir.to_string_lossy().to_string(),
        "--username".to_string(),
        username.to_string(),
        "--pwfile".to_string(),
        pwfile.to_string_lossy().to_string(),
        "--auth-local=trust".to_string(),
        "--auth-host=scram-sha-256".to_string(),
        "--encoding=UTF8".to_string(),
        "--locale=C".to_string(),
    ]
}

/// Argumen menjalankan server PostgreSQL (tanpa argv[0]).
///
/// argv[0] harus `postgres_binary_path(home)` (symlink), bukan path
/// libpostgres.so langsung. Dua override `-c` mmap adalah pengaman kedua
/// bila postgresql.conf tertimpa file bawaan initdb.
pub fn postgres_start_args(home: &Path) -> Vec<String> {
    vec![
        "-D".to_string(),
        postgres_data_dir(home).to_string_lossy().to_string(),
        "-c".to_string(),
        format!(
            "config_file={}",
            postgres_conf_path(home).to_string_lossy()
        ),
        "-c".to_string(),
        "shared_memory_type=mmap".to_string(),
        "-c".to_string(),
        "dynamic_shared_memory_type=mmap".to_string(),
    ]
}

/// Argumen pg_ctl untuk menghentikan server (tanpa argv[0]).
/// Mode `fast`: koneksi aktif diputus, checkpoint bersih tetap dilakukan.
pub fn postgres_stop_args(home: &Path) -> Vec<String> {
    vec![
        "-D".to_string(),
        postgres_data_dir(home).to_string_lossy().to_string(),
        "-m".to_string(),
        "fast".to_string(),
        "-w".to_string(),
        "-t".to_string(),
        "30".to_string(),
        "stop".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// MariaDB: path turunan
// ---------------------------------------------------------------------------

pub fn mariadb_data_dir(home: &Path) -> PathBuf {
    home.join("data")
}

pub fn mariadb_run_dir(home: &Path) -> PathBuf {
    home.join("run")
}

pub fn mariadb_tmp_dir(home: &Path) -> PathBuf {
    home.join("tmp")
}

/// Direktori share MariaDB hasil ekstrak mariadb-share.zip. Berisi
/// `charsets/`, `english/errmsg.sys`, dan file SQL sistem (`*.sql`).
pub fn mariadb_share_dir(home: &Path) -> PathBuf {
    home.join("share").join("mysql")
}

pub fn mariadb_cnf_path(home: &Path) -> PathBuf {
    home.join("my.cnf")
}

pub fn mariadb_socket_path(home: &Path) -> PathBuf {
    mariadb_run_dir(home).join("mysqld.sock")
}

pub fn mariadb_pid_path(home: &Path) -> PathBuf {
    mariadb_run_dir(home).join("mariadbd.pid")
}

/// File log error MariaDB. Core membaca file ini ke log center
/// (MariaDB tidak menulis log error ke stderr secara andal).
pub fn mariadb_error_log_path(home: &Path) -> PathBuf {
    home.join("error.log")
}

/// Binary server MariaDB di nativeLibraryDir. Berbeda dengan PostgreSQL,
/// MariaDB tidak perlu symlink argv[0]: basedir/datadir/share semuanya
/// eksplisit di my.cnf.
pub fn mariadb_binary_path(native_lib_dir: &Path) -> PathBuf {
    native_lib_dir.join("libmariadbd.so")
}

/// Direktori yang harus dibuat core sebelum bootstrap/start MariaDB.
pub fn mariadb_required_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.to_path_buf(),
        mariadb_data_dir(home),
        mariadb_run_dir(home),
        mariadb_tmp_dir(home),
    ]
}

// ---------------------------------------------------------------------------
// MariaDB: config & argumen
// ---------------------------------------------------------------------------

/// Isi my.cnf untuk instance lokal di Android.
///
/// Hanya 127.0.0.1, buffer pool kecil untuk ponsel, InnoDB native AIO
/// dimatikan (Android tanpa libaio), log error ke file di home instance.
pub fn generate_mariadb_cnf(port: u16, home: &Path, share_dir: &Path) -> String {
    format!(
        "# Dihasilkan WebForge Mobile core (db_runtime.rs). Jangan edit manual;\n\
         # file ini ditimpa core setiap instance disiapkan.\n\
         [mysqld]\n\
         basedir = {home}\n\
         datadir = {data}\n\
         socket = {socket}\n\
         pid-file = {pid}\n\
         port = {port}\n\
         bind-address = 127.0.0.1\n\
         tmpdir = {tmp}\n\
         character-sets-dir = {charsets}\n\
         lc-messages-dir = {share}\n\
         lc-messages = en_US\n\
         skip-name-resolve\n\
         innodb_buffer_pool_size = 64M\n\
         innodb_use_native_aio = OFF\n\
         log-error = {error_log}\n\
         max_connections = 20\n\
         character-set-server = utf8mb4\n\
         collation-server = utf8mb4_unicode_ci\n\
         default-storage-engine = InnoDB\n",
        home = home.to_string_lossy(),
        data = mariadb_data_dir(home).to_string_lossy(),
        socket = mariadb_socket_path(home).to_string_lossy(),
        pid = mariadb_pid_path(home).to_string_lossy(),
        port = port,
        tmp = mariadb_tmp_dir(home).to_string_lossy(),
        charsets = share_dir.join("charsets").to_string_lossy(),
        share = share_dir.to_string_lossy(),
        error_log = mariadb_error_log_path(home).to_string_lossy(),
    )
}

/// Argumen mariadb-install-db (tanpa argv[0]) untuk lingkungan yang
/// menyediakan skrip tersebut. Di Android jalur utamanya adalah
/// `mariadb_bootstrap_args` karena mariadb-install-db adalah skrip shell.
pub fn mariadb_install_args(home: &Path, share_dir: &Path) -> Vec<String> {
    vec![
        format!("--basedir={}", home.to_string_lossy()),
        format!("--datadir={}", mariadb_data_dir(home).to_string_lossy()),
        format!(
            "--lc-messages-dir={}",
            share_dir.to_string_lossy()
        ),
        "--auth-root-authentication-method=normal".to_string(),
        "--skip-test-db".to_string(),
    ]
}

/// Argumen `mariadbd --bootstrap` (tanpa argv[0]): server membaca SQL
/// inisialisasi dari stdin tanpa membuka port. Koordinator mengalirkan
/// isi `mariadb_bootstrap_sql_files` berurutan, lalu SQL penetapan
/// password root (`mariadb_set_root_password_sql`), ke stdin proses ini.
pub fn mariadb_bootstrap_args(home: &Path) -> Vec<String> {
    vec![
        format!(
            "--defaults-file={}",
            mariadb_cnf_path(home).to_string_lossy()
        ),
        "--bootstrap".to_string(),
    ]
}

/// File SQL sistem MariaDB 10.11 yang harus dialirkan ke
/// `mariadbd --bootstrap`, dalam urutan yang dipakai mariadb-install-db.
/// Isi file-file ini tidak memilih database sendiri untuk tabel sistem
/// utama, jadi rakit SQL bootstrap wajib memakai
/// [`assemble_mariadb_bootstrap_sql`] (membuat + memilih database
/// `mysql` lebih dulu), bukan menggabungkan file mentah begitu saja.
pub fn mariadb_bootstrap_sql_files(share_dir: &Path) -> Vec<PathBuf> {
    [
        "mysql_system_tables.sql",
        "mysql_system_tables_data.sql",
        "mysql_performance_tables.sql",
        "mysql_sys_schema.sql",
        "fill_help_tables.sql",
    ]
    .iter()
    .map(|name| share_dir.join(name))
    .collect()
}

/// Rakit SQL bootstrap MariaDB dari isi file sistem + password root.
///
/// File `mysql_system_tables.sql` langsung membuat tabel tanpa
/// kualifikasi skema dan mengasumsikan database `mysql` sudah terpilih
/// (mariadb-install-db menyiapkan konteks itu). Tanpa awalan ini
/// bootstrap gagal `ERROR 1046 No database selected`. Awalan membuat
/// database `mysql` bila belum ada lalu memilihnya; `USE mysql`
/// diulang sebelum penetapan password karena skrip performance/sys di
/// tengah dapat mengubah konteks database sesi.
pub fn assemble_mariadb_bootstrap_sql(file_contents: &[String], password: &str) -> String {
    let mut sql = String::from("CREATE DATABASE IF NOT EXISTS mysql;\nUSE mysql;\n");
    for content in file_contents {
        sql.push_str(content);
        if !content.ends_with('\n') {
            sql.push('\n');
        }
    }
    sql.push_str("USE mysql;\n");
    sql.push_str(&mariadb_set_root_password_sql(password));
    sql
}

/// SQL penetapan password root setelah bootstrap. Data sistem membuat
/// akun root tanpa password untuk localhost, hostname, 127.0.0.1, dan
/// ::1. Core menghubungi MariaDB lewat TCP 127.0.0.1, jadi password
/// wajib dipasang pada root@127.0.0.1 juga (bukan hanya localhost
/// socket), dan akun root host lain dihapus agar tidak tertinggal
/// tanpa password. SQL ini dijalankan dalam sesi bootstrap yang sama
/// sebelum server melayani koneksi TCP.
pub fn mariadb_set_root_password_sql(password: &str) -> String {
    let escaped = password.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        "DELETE FROM mysql.global_priv WHERE User='root' AND Host NOT IN ('localhost','127.0.0.1');\n\
         ALTER USER 'root'@'localhost' IDENTIFIED BY '{escaped}';\n\
         ALTER USER 'root'@'127.0.0.1' IDENTIFIED BY '{escaped}';\n\
         FLUSH PRIVILEGES;\n",
        escaped = escaped
    )
}

/// Argumen menjalankan server MariaDB (tanpa argv[0]).
/// Penghentian dilakukan core dengan SIGTERM ke proses (tidak ada
/// mariadb-admin di bundle); mariadbd menutup InnoDB secara bersih.
pub fn mariadb_start_args(home: &Path) -> Vec<String> {
    vec![format!(
        "--defaults-file={}",
        mariadb_cnf_path(home).to_string_lossy()
    )]
}

// ---------------------------------------------------------------------------
// Parsing versi
// ---------------------------------------------------------------------------

fn looks_like_version(token: &str) -> bool {
    let mut parts = token.split('.');
    let first = parts.next().unwrap_or("");
    !first.is_empty()
        && first.chars().all(|c| c.is_ascii_digit())
        && parts.next().is_some_and(|p| {
            !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())
        })
}

fn clean_version_token(token: &str) -> String {
    token
        .trim()
        .trim_end_matches([',', ';', ')'])
        .to_string()
}

/// Parse versi dari output `postgres --version` / `psql --version` /
/// `SELECT version()`, mis. "postgres (PostgreSQL) 16.3" -> "16.3".
pub fn parse_postgres_version(output: &str) -> Option<String> {
    let tokens: Vec<&str> = output.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        if *token == "(PostgreSQL)" || *token == "PostgreSQL" {
            if let Some(next) = tokens.get(i + 1) {
                let v = clean_version_token(next);
                if looks_like_version(&v) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Parse versi dari output `mariadbd --version`, mis.
/// "mariadbd  Ver 10.11.8-MariaDB for linux" -> "10.11.8".
pub fn parse_mariadb_version(output: &str) -> Option<String> {
    let tokens: Vec<&str> = output.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        if *token == "Ver" {
            if let Some(next) = tokens.get(i + 1) {
                let v = clean_version_token(next);
                let v = v.split('-').next().unwrap_or("").to_string();
                if looks_like_version(&v) {
                    return Some(v);
                }
            }
        }
    }
    // Fallback: token pertama berbentuk x.y.z (mis. output "10.11.8-MariaDB").
    for token in tokens {
        let v = clean_version_token(token);
        let v = v.split('-').next().unwrap_or("").to_string();
        if looks_like_version(&v) && v.split('.').count() >= 3 {
            return Some(v);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Validasi identifier
// ---------------------------------------------------------------------------

/// Validasi nama database/user: huruf ASCII, angka, dan underscore,
/// diawali huruf atau underscore, maks 63 karakter (batas identifier
/// PostgreSQL). Aturan ketat ini menolak injeksi lewat nama yang nantinya
/// disisipkan ke SQL DDL oleh koordinator.
pub fn validate_db_identifier(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Nama tidak boleh kosong".to_string());
    }
    if name.len() > 63 {
        return Err("Nama maksimal 63 karakter".to_string());
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err("Nama harus diawali huruf atau underscore".to_string());
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(
            "Nama hanya boleh berisi huruf, angka, dan underscore".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_pg() -> PathBuf {
        postgres_home(Path::new("/data/app"), "inst-1")
    }

    fn home_my() -> PathBuf {
        mariadb_home(Path::new("/data/app"), "inst-1")
    }

    #[test]
    fn home_paths_under_databases_services() {
        assert_eq!(
            home_pg(),
            PathBuf::from("/data/app/databases-services/inst-1/postgres")
        );
        assert_eq!(
            home_my(),
            PathBuf::from("/data/app/databases-services/inst-1/mariadb")
        );
    }

    #[test]
    fn postgres_conf_has_port_bind_socket_and_mmap() {
        let conf = generate_postgresql_conf(55432, &home_pg());
        assert!(conf.contains("port = 55432"));
        assert!(conf.contains("listen_addresses = '127.0.0.1'"));
        assert!(conf.contains(
            "unix_socket_directories = '/data/app/databases-services/inst-1/postgres/run'"
        ));
        assert!(conf.contains("dynamic_shared_memory_type = mmap"));
        assert!(conf.contains("shared_memory_type = mmap"));
        assert!(conf.contains("shared_buffers = '32MB'"));
        assert!(conf.contains("max_connections = 20"));
        assert!(conf.contains("logging_collector = off"));
        assert!(conf.contains("log_destination = 'stderr'"));
        assert!(conf.contains("timezone = 'UTC'"));
    }

    #[test]
    fn postgres_init_args_use_pwfile_and_scram_host_auth() {
        let home = home_pg();
        let args = postgres_init_args(
            &home,
            &postgres_share_dir(&home),
            "webforge",
            Path::new("/data/app/pw.txt"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-D /data/app/databases-services/inst-1/postgres/data"));
        assert!(joined.contains("-L /data/app/databases-services/inst-1/postgres/share/postgresql"));
        assert!(joined.contains("--username webforge"));
        assert!(joined.contains("--pwfile /data/app/pw.txt"));
        assert!(joined.contains("--auth-local=trust"));
        assert!(joined.contains("--auth-host=scram-sha-256"));
        assert!(joined.contains("--encoding=UTF8"));
        assert!(joined.contains("--locale=C"));
    }

    #[test]
    fn postgres_start_and_stop_args() {
        let home = home_pg();
        let start = postgres_start_args(&home).join(" ");
        assert!(start.contains("-D /data/app/databases-services/inst-1/postgres/data"));
        assert!(start.contains("config_file=/data/app/databases-services/inst-1/postgres/data/postgresql.conf"));
        assert!(start.contains("dynamic_shared_memory_type=mmap"));

        let stop = postgres_stop_args(&home).join(" ");
        assert!(stop.contains("-m fast"));
        assert!(stop.ends_with("stop"));
    }

    #[test]
    fn postgres_symlinks_point_to_native_libs() {
        let home = home_pg();
        let links = postgres_symlinks(&home, Path::new("/data/app/lib/arm64"));
        assert_eq!(links.len(), 4);
        assert_eq!(
            links[0],
            (
                PathBuf::from("/data/app/databases-services/inst-1/postgres/bin/postgres"),
                PathBuf::from("/data/app/lib/arm64/libpostgres.so")
            )
        );
        assert!(links.iter().any(|(_, t)| t.ends_with("libpgctl.so")));
    }

    #[cfg(unix)]
    #[test]
    fn postgres_symlinks_are_replaced_when_native_dir_changes() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "wf-pg-symlink-{}-{}",
            std::process::id(),
            nanos
        ));
        let home = root.join("home");
        let old_native = root.join("old-lib");
        let new_native = root.join("new-lib");
        std::fs::create_dir_all(&old_native).unwrap();
        std::fs::create_dir_all(&new_native).unwrap();
        for dir in [&old_native, &new_native] {
            for name in ["libpostgres.so", "libinitdb.so", "libpgctl.so", "libpsql.so"] {
                std::fs::write(dir.join(name), b"elf").unwrap();
            }
        }
        ensure_postgres_symlinks(&home, &old_native).unwrap();
        assert_eq!(
            std::fs::read_link(postgres_initdb_path(&home)).unwrap(),
            old_native.join("libinitdb.so")
        );
        ensure_postgres_symlinks(&home, &new_native).unwrap();
        assert_eq!(
            std::fs::read_link(postgres_initdb_path(&home)).unwrap(),
            new_native.join("libinitdb.so")
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mariadb_cnf_has_port_bind_socket_and_innodb() {
        let home = home_my();
        let cnf = generate_mariadb_cnf(53306, &home, &mariadb_share_dir(&home));
        assert!(cnf.contains("[mysqld]"));
        assert!(cnf.contains("port = 53306"));
        assert!(cnf.contains("bind-address = 127.0.0.1"));
        assert!(cnf.contains(
            "socket = /data/app/databases-services/inst-1/mariadb/run/mysqld.sock"
        ));
        assert!(cnf.contains("datadir = /data/app/databases-services/inst-1/mariadb/data"));
        assert!(cnf.contains(
            "character-sets-dir = /data/app/databases-services/inst-1/mariadb/share/mysql/charsets"
        ));
        assert!(cnf.contains("skip-name-resolve"));
        assert!(cnf.contains("innodb_buffer_pool_size = 64M"));
        assert!(cnf.contains("innodb_use_native_aio = OFF"));
        assert!(cnf.contains(
            "log-error = /data/app/databases-services/inst-1/mariadb/error.log"
        ));
    }

    #[test]
    fn mariadb_install_bootstrap_and_start_args() {
        let home = home_my();
        let install =
            mariadb_install_args(&home, &mariadb_share_dir(&home)).join(" ");
        assert!(install.contains("--basedir=/data/app/databases-services/inst-1/mariadb"));
        assert!(install.contains("--datadir=/data/app/databases-services/inst-1/mariadb/data"));
        assert!(install.contains("--auth-root-authentication-method=normal"));
        assert!(install.contains("--skip-test-db"));

        let bootstrap = mariadb_bootstrap_args(&home).join(" ");
        assert!(bootstrap.contains("--defaults-file=/data/app/databases-services/inst-1/mariadb/my.cnf"));
        assert!(bootstrap.contains("--bootstrap"));

        let files = mariadb_bootstrap_sql_files(&mariadb_share_dir(&home));
        assert_eq!(files.len(), 5);
        assert!(files[0].ends_with("mysql_system_tables.sql"));
        assert!(files[4].ends_with("fill_help_tables.sql"));

        let start = mariadb_start_args(&home).join(" ");
        assert!(start.contains("--defaults-file=/data/app/databases-services/inst-1/mariadb/my.cnf"));
    }

    #[test]
    fn mariadb_root_password_sql_escapes_quotes() {
        let sql = mariadb_set_root_password_sql("a'b\\c");
        assert!(sql.contains("ALTER USER 'root'@'localhost' IDENTIFIED BY 'a\\'b\\\\c'"));
        assert!(sql.contains("ALTER USER 'root'@'127.0.0.1' IDENTIFIED BY 'a\\'b\\\\c'"));
        assert!(sql.contains("DELETE FROM mysql.global_priv WHERE User='root'"));
        assert!(sql.contains("FLUSH PRIVILEGES"));
    }

    #[test]
    fn mariadb_bootstrap_sql_selects_mysql_before_system_tables() {
        let contents = vec![
            "CREATE TABLE db (Host char(255));\n".to_string(),
            "INSERT INTO global_priv VALUES ('localhost','root','{}');\n".to_string(),
        ];
        let sql = assemble_mariadb_bootstrap_sql(&contents, "secret");
        let create_pos = sql.find("CREATE DATABASE IF NOT EXISTS mysql;").unwrap();
        let use_pos = sql.find("USE mysql;").unwrap();
        let table_pos = sql.find("CREATE TABLE db").unwrap();
        assert!(create_pos < use_pos && use_pos < table_pos, "sql: {}", sql);
        let alter_pos = sql.find("ALTER USER 'root'@'127.0.0.1'").unwrap();
        let insert_pos = sql.find("INSERT INTO global_priv").unwrap();
        assert!(insert_pos < alter_pos, "sql: {}", sql);
        assert!(sql[insert_pos..alter_pos].contains("USE mysql;"), "sql: {}", sql);
    }

    #[test]
    fn parse_versions() {
        assert_eq!(
            parse_postgres_version("postgres (PostgreSQL) 16.3"),
            Some("16.3".to_string())
        );
        assert_eq!(
            parse_postgres_version("psql (PostgreSQL) 16.3 on aarch64"),
            Some("16.3".to_string())
        );
        assert_eq!(
            parse_postgres_version("PostgreSQL 16.3 on aarch64-unknown-linux-android"),
            Some("16.3".to_string())
        );
        assert_eq!(parse_postgres_version("bukan versi"), None);

        assert_eq!(
            parse_mariadb_version("mariadbd  Ver 10.11.8-MariaDB for linux on aarch64"),
            Some("10.11.8".to_string())
        );
        assert_eq!(
            parse_mariadb_version("10.11.8-MariaDB"),
            Some("10.11.8".to_string())
        );
        assert_eq!(parse_mariadb_version("bukan versi"), None);
    }

    #[test]
    fn identifier_validation_accepts_safe_names() {
        assert!(validate_db_identifier("webforge_db1").is_ok());
        assert!(validate_db_identifier("_x").is_ok());
        assert!(validate_db_identifier(&"a".repeat(63)).is_ok());
    }

    #[test]
    fn identifier_validation_rejects_injection_and_unsafe_names() {
        for bad in [
            "",
            "9abc",
            "a b",
            "a-b",
            "a.b",
            "a;DROP TABLE users",
            "a'b",
            "a\"b",
            "a`b",
            "db--comment",
            "é",
            "a/b",
            "a\\b",
        ] {
            assert!(
                validate_db_identifier(bad).is_err(),
                "seharusnya ditolak: {:?}",
                bad
            );
        }
        assert!(validate_db_identifier(&"a".repeat(64)).is_err());
    }
}
