//! Python runtime: deteksi binary nyata, lingkungan proses, deteksi entry
//! framework, dan parsing kebutuhan paket.
//!
//! Binary Python di Android adalah `libpython.so` (CPython CLI hasil
//! cross-compile, dikemas sebagai .so agar sah dieksekusi dari
//! nativeLibraryDir — W^X). Di host/test, `python3`/`python` dari PATH
//! dipakai bila ada. Modul ini hanya menyusun argumen dan environment;
//! ia tidak mengklaim Flask/FastAPI/Django terinstal. Bila modul framework
//! tidak ada, proses Python akan gagal nyata dan log proses menunjukkannya.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Cari binary Python nyata: `libpython.so` di nativeLibraryDir lebih dulu
/// (bundled Android), lalu `python3`, lalu `python` di PATH (host/test).
pub fn find_python_binary(native_lib_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = native_lib_dir {
        let cand = dir.join("libpython.so");
        if cand.is_file() {
            return Some(cand);
        }
    }
    crate::runtime::find_in_path("python3").or_else(|| crate::runtime::find_in_path("python"))
}

/// true bila binary berasal dari bundle (libpython.so), bukan PATH.
pub fn is_bundled_python(binary: &Path, native_lib_dir: Option<&Path>) -> bool {
    match native_lib_dir {
        Some(dir) => binary == dir.join("libpython.so"),
        None => false,
    }
}

/// PYTHONHOME untuk interpreter bundled: `<data_dir>/python`.
pub fn python_home(data_dir: &Path) -> PathBuf {
    data_dir.join("python")
}

/// Zip stdlib yang disalin PythonAssets: `<data_dir>/python/stdlib.zip`.
pub fn stdlib_zip_path(data_dir: &Path) -> PathBuf {
    python_home(data_dir).join("stdlib.zip")
}

/// Folder hasil ekstrak stdlib oleh PythonAssets:
/// `<data_dir>/python/lib/python3.12` (layout PYTHONHOME CPython).
pub fn stdlib_dir_path(data_dir: &Path) -> PathBuf {
    python_home(data_dir).join("lib").join("python3.12")
}

fn sanitize_project_id(project_id: &str) -> String {
    let cleaned: String = project_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Folder venv per proyek: `<data_dir>/venvs/<project_id>`.
/// project_id disanitasi agar tidak bisa keluar dari folder venvs.
pub fn venv_dir(data_dir: &Path, project_id: &str) -> PathBuf {
    data_dir.join("venvs").join(sanitize_project_id(project_id))
}

/// Cari site-packages venv yang benar-benar ada. Venv CPython standar
/// memakai `lib/pythonX.Y/site-packages`; fallback `pip --target` memakai
/// `site-packages` langsung di folder venv.
pub fn venv_site_packages(data_dir: &Path, project_id: &str) -> Option<PathBuf> {
    let venv = venv_dir(data_dir, project_id);
    if !venv.is_dir() {
        return None;
    }
    let lib_dir = venv.join("lib");
    if let Ok(entries) = std::fs::read_dir(&lib_dir) {
        let mut candidates: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("python"))
                        .unwrap_or(false)
            })
            .map(|p| p.join("site-packages"))
            .filter(|p| p.is_dir())
            .collect();
        candidates.sort();
        if let Some(found) = candidates.pop() {
            return Some(found);
        }
    }
    let fallback = venv.join("site-packages");
    if fallback.is_dir() {
        Some(fallback)
    } else {
        None
    }
}

/// Environment untuk proses Python. Koordinator meneruskannya apa adanya
/// ke ChildLaunch.envs. Folder HOME/TMPDIR berada di dalam data_dir karena
/// Android tidak menyediakan /tmp yang bisa ditulis aplikasi.
pub fn python_env(data_dir: &Path, project_id: &str) -> Vec<(String, String)> {
    // Stdlib filesystem hasil ekstrak (bukan zip) agar ensurepip/venv
    // menemukan file nyata lewat __file__.
    let mut pythonpath_parts = vec![stdlib_dir_path(data_dir).to_string_lossy().to_string()];
    let venv = venv_dir(data_dir, project_id);
    let venv_exists = venv.is_dir();
    if let Some(site_packages) = venv_site_packages(data_dir, project_id) {
        pythonpath_parts.push(site_packages.to_string_lossy().to_string());
    }

    let mut env = vec![
        (
            "PYTHONHOME".to_string(),
            python_home(data_dir).to_string_lossy().to_string(),
        ),
        ("PYTHONPATH".to_string(), pythonpath_parts.join(":")),
        (
            "HOME".to_string(),
            data_dir.to_string_lossy().to_string(),
        ),
        (
            "TMPDIR".to_string(),
            data_dir.join("tmp").to_string_lossy().to_string(),
        ),
        ("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string()),
        (
            "PIP_DISABLE_PIP_VERSION_CHECK".to_string(),
            "1".to_string(),
        ),
    ];
    if venv_exists {
        env.push((
            "VIRTUAL_ENV".to_string(),
            venv.to_string_lossy().to_string(),
        ));
    }
    env
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PythonEntryKind {
    Django,
    Flask,
    FastApi,
    HttpServer,
    Script,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonEntry {
    pub kind: PythonEntryKind,
    /// File entry relatif terhadap project_path, mis. "manage.py" atau
    /// "main.py". Untuk HttpServer nilainya "http.server".
    pub entry: String,
    /// Target aplikasi: Flask = nama modul, FastAPI = "modul:variabel".
    pub app_target: Option<String>,
}

fn read_project_file(project_path: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(project_path.join(rel)).ok()
}

fn module_name(file_name: &str) -> String {
    file_name
        .strip_suffix(".py")
        .unwrap_or(file_name)
        .to_string()
}

fn find_assignment_var(content: &str, constructor: &str) -> Option<String> {
    for line in content.lines() {
        let t = line.trim();
        if !t.contains(constructor) {
            continue;
        }
        if let Some((lhs, _rhs)) = t.split_once('=') {
            let var = lhs.trim();
            if !var.is_empty()
                && var
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !var.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            {
                return Some(var.to_string());
            }
        }
    }
    None
}

fn content_mentions_fastapi(content: &str) -> bool {
    content.contains("FastAPI(")
        || content.contains("from fastapi import")
        || content.contains("import fastapi")
}

fn content_mentions_flask(content: &str) -> bool {
    content.contains("Flask(")
        || content.contains("from flask import")
        || content.contains("import flask")
}

fn python_dependency_names(project_path: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(text) = read_project_file(project_path, "requirements.txt") {
        names.extend(parse_requirements(&text));
    }
    if let Some(text) = read_project_file(project_path, "pyproject.toml") {
        // Scan kasar string dependency seperti "django>=5.0" di pyproject.
        for line in text.lines() {
            let mut rest = line;
            while let Some(start) = rest.find('"') {
                let after = &rest[start + 1..];
                let Some(end_rel) = after.find('"') else { break };
                let inner = &after[..end_rel];
                let name_end = inner
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                    .unwrap_or(inner.len());
                let name = inner[..name_end].to_lowercase().replace('_', "-");
                if ["django", "flask", "fastapi", "uvicorn"].contains(&name.as_str()) {
                    names.push(name);
                }
                rest = &after[end_rel + 1..];
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Deteksi entry Python dari file proyek nyata. Urutan: Django
/// (manage.py), FastAPI, Flask, script Python biasa, lalu http.server.
pub fn detect_python_entry(project_path: &Path) -> PythonEntry {
    if project_path.join("manage.py").is_file() {
        return PythonEntry {
            kind: PythonEntryKind::Django,
            entry: "manage.py".to_string(),
            app_target: None,
        };
    }

    let deps = python_dependency_names(project_path);
    let has_fastapi_dep = deps.iter().any(|d| d == "fastapi");
    let has_flask_dep = deps.iter().any(|d| d == "flask");

    // FastAPI lebih dulu: main.py/app.py/asgi.py adalah pola umum uvicorn.
    for file in ["main.py", "app.py", "asgi.py", "server.py", "run.py"] {
        if let Some(content) = read_project_file(project_path, file) {
            if content_mentions_fastapi(&content) || (has_fastapi_dep && file == "main.py") {
                let var = find_assignment_var(&content, "FastAPI(")
                    .unwrap_or_else(|| "app".to_string());
                return PythonEntry {
                    kind: PythonEntryKind::FastApi,
                    entry: file.to_string(),
                    app_target: Some(format!("{}:{}", module_name(file), var)),
                };
            }
        }
    }

    for file in ["app.py", "main.py", "wsgi.py", "server.py", "run.py"] {
        if let Some(content) = read_project_file(project_path, file) {
            if content_mentions_flask(&content) || (has_flask_dep && file == "app.py") {
                return PythonEntry {
                    kind: PythonEntryKind::Flask,
                    entry: file.to_string(),
                    app_target: Some(module_name(file)),
                };
            }
        }
    }

    for file in ["app.py", "main.py", "server.py", "run.py", "wsgi.py", "asgi.py"] {
        if project_path.join(file).is_file() {
            return PythonEntry {
                kind: PythonEntryKind::Script,
                entry: file.to_string(),
                app_target: None,
            };
        }
    }

    PythonEntry {
        kind: PythonEntryKind::HttpServer,
        entry: "http.server".to_string(),
        app_target: None,
    }
}

/// Argumen server Python. Proses dijalankan dengan cwd = project_path oleh
/// koordinator. Bila modul flask/uvicorn/django belum terinstal, proses
/// akan gagal nyata (ModuleNotFoundError di log), bukan pura-pura jalan.
pub fn build_python_server_args(
    entry: &PythonEntry,
    host: &str,
    port: u16,
    project_path: &Path,
) -> Vec<String> {
    match entry.kind {
        PythonEntryKind::Django => vec![
            "manage.py".to_string(),
            "runserver".to_string(),
            format!("{}:{}", host, port),
            "--noreload".to_string(),
        ],
        PythonEntryKind::Flask => {
            let module = entry
                .app_target
                .clone()
                .unwrap_or_else(|| module_name(&entry.entry));
            vec![
                "-m".to_string(),
                "flask".to_string(),
                "--app".to_string(),
                module,
                "run".to_string(),
                "--host".to_string(),
                host.to_string(),
                "--port".to_string(),
                port.to_string(),
            ]
        }
        PythonEntryKind::FastApi => {
            let target = entry
                .app_target
                .clone()
                .unwrap_or_else(|| format!("{}:app", module_name(&entry.entry)));
            vec![
                "-m".to_string(),
                "uvicorn".to_string(),
                target,
                "--host".to_string(),
                host.to_string(),
                "--port".to_string(),
                port.to_string(),
            ]
        }
        PythonEntryKind::HttpServer => vec![
            "-m".to_string(),
            "http.server".to_string(),
            port.to_string(),
            "--bind".to_string(),
            host.to_string(),
            "--directory".to_string(),
            project_path.to_string_lossy().to_string(),
        ],
        PythonEntryKind::Script => {
            // Script biasa tidak punya konvensi argumen host/port. Jalankan
            // apa adanya; koordinator menyediakan PORT/HOST lewat env bila
            // script membacanya. Bila script tidak membuka port, health
            // check akan gagal secara nyata.
            vec![project_path.join(&entry.entry).to_string_lossy().to_string()]
        }
    }
}

/// Argumen membuat venv standar (dengan pip dari ensurepip).
///
/// Keputusan Android: TIDAK memakai --copies. --copies menyalin binary
/// interpreter ke folder data aplikasi yang writable; file hasil salinan
/// tidak sah dieksekusi di bawah W^X/SELinux Android. Venv standar membuat
/// symlink bin/python ke libpython.so di nativeLibraryDir. Bila symlink
/// exec terbukti bermasalah di perangkat nyata, fallback-nya adalah
/// site-packages terisolasi lewat build_pip_install_args (pip --target)
/// dan interpreter base dijalankan langsung dengan PYTHONPATH ke folder
/// target itu.
pub fn build_venv_args(venv_dir: &Path) -> Vec<String> {
    vec![
        "-m".to_string(),
        "venv".to_string(),
        venv_dir.to_string_lossy().to_string(),
    ]
}

/// Argumen fallback instalasi dependensi tanpa venv executable:
/// pip memasang paket ke target_dir, lalu python_env menambahkan
/// target_dir (venv/site-packages) ke PYTHONPATH.
pub fn build_pip_install_args(target_dir: &Path, requirements_path: &Path) -> Vec<String> {
    vec![
        "-m".to_string(),
        "pip".to_string(),
        "install".to_string(),
        "--target".to_string(),
        target_dir.to_string_lossy().to_string(),
        "-r".to_string(),
        requirements_path.to_string_lossy().to_string(),
    ]
}

/// Parse nama paket kasar dari requirements.txt. Komentar, baris kosong,
/// dan opsi pip (termasuk -r) diabaikan. Nama dinormalisasi lowercase
/// dengan '_' menjadi '-'.
pub fn parse_requirements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        if let Some(idx) = line.find(" #") {
            line = line[..idx].trim();
        }
        if let Some(idx) = line.find(';') {
            line = line[..idx].trim();
        }
        if line.is_empty() {
            continue;
        }
        let name_part = if let Some(idx) = line.find(" @ ") {
            line[..idx].trim()
        } else {
            line
        };
        let name_end = name_part
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
            .unwrap_or(name_part.len());
        let name = name_part[..name_end].to_lowercase().replace('_', "-");
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

/// Parse output `pip list --format=json` menjadi (name, version).
/// Output tidak valid menghasilkan Vec kosong, bukan tebakan.
pub fn parse_pip_list(output: &str) -> Vec<(String, String)> {
    let parsed = serde_json::from_str::<serde_json::Value>(output).or_else(|_| {
        let start = output.find('[').ok_or_else(|| "no json array".to_string())?;
        let end = output.rfind(']').ok_or_else(|| "no json array".to_string())?;
        if end <= start {
            return Err("no json array".to_string());
        }
        serde_json::from_str::<serde_json::Value>(&output[start..=end])
            .map_err(|e| e.to_string())
    });
    let Ok(value) = parsed else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let version = item.get("version").and_then(|v| v.as_str()).unwrap_or("");
        if !name.is_empty() {
            out.push((name.to_string(), version.to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wf-py-test-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (rel, content) in files {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, content).unwrap();
        }
        dir
    }

    #[test]
    fn paths_and_env_follow_data_dir_layout() {
        let data = Path::new("/data/webforge");
        assert_eq!(python_home(data), PathBuf::from("/data/webforge/python"));
        assert_eq!(
            stdlib_zip_path(data),
            PathBuf::from("/data/webforge/python/stdlib.zip")
        );
        assert_eq!(
            venv_dir(data, "proj-1"),
            PathBuf::from("/data/webforge/venvs/proj-1")
        );
        let env = python_env(data, "proj-1");
        assert!(env.contains(&(
            "PYTHONHOME".to_string(),
            "/data/webforge/python".to_string()
        )));
        assert!(env.contains(&(
            "PYTHONPATH".to_string(),
            "/data/webforge/python/lib/python3.12".to_string()
        )));
        assert!(env.contains(&("HOME".to_string(), "/data/webforge".to_string())));
        assert!(env.contains(&(
            "TMPDIR".to_string(),
            "/data/webforge/tmp".to_string()
        )));
        assert!(env.contains(&("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string())));
        assert!(!env.iter().any(|(k, _)| k == "VIRTUAL_ENV"));
    }

    #[test]
    fn env_includes_venv_site_packages_when_present() {
        let data = fixture(&[]);
        let site = data
            .join("venvs")
            .join("p1")
            .join("lib")
            .join("python3.12")
            .join("site-packages");
        std::fs::create_dir_all(&site).unwrap();
        let env = python_env(&data, "p1");
        let pythonpath = env
            .iter()
            .find(|(k, _)| k == "PYTHONPATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert!(pythonpath.contains("lib/python3.12"));
        assert!(pythonpath.contains(&site.to_string_lossy().to_string()));
        assert!(env.iter().any(|(k, _)| k == "VIRTUAL_ENV"));
        std::fs::remove_dir_all(data).ok();
    }

    #[test]
    fn detects_django_flask_fastapi_script_and_http() {
        let d = fixture(&[("manage.py", "# django"), ("requirements.txt", "Django==5.0\n")]);
        let e = detect_python_entry(&d);
        assert_eq!(e.kind, PythonEntryKind::Django);
        assert_eq!(e.entry, "manage.py");
        std::fs::remove_dir_all(&d).ok();

        let d = fixture(&[("app.py", "from flask import Flask\napp = Flask(__name__)\n")]);
        let e = detect_python_entry(&d);
        assert_eq!(e.kind, PythonEntryKind::Flask);
        assert_eq!(e.app_target.as_deref(), Some("app"));
        std::fs::remove_dir_all(&d).ok();

        let d = fixture(&[(
            "main.py",
            "from fastapi import FastAPI\napi = FastAPI()\n",
        )]);
        let e = detect_python_entry(&d);
        assert_eq!(e.kind, PythonEntryKind::FastApi);
        assert_eq!(e.app_target.as_deref(), Some("main:api"));
        std::fs::remove_dir_all(&d).ok();

        let d = fixture(&[("server.py", "print('halo')\n")]);
        let e = detect_python_entry(&d);
        assert_eq!(e.kind, PythonEntryKind::Script);
        assert_eq!(e.entry, "server.py");
        std::fs::remove_dir_all(&d).ok();

        let d = fixture(&[("index.html", "<html></html>")]);
        let e = detect_python_entry(&d);
        assert_eq!(e.kind, PythonEntryKind::HttpServer);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn server_args_per_framework() {
        let root = Path::new("/data/workspace/site");
        let django = PythonEntry {
            kind: PythonEntryKind::Django,
            entry: "manage.py".to_string(),
            app_target: None,
        };
        assert_eq!(
            build_python_server_args(&django, "127.0.0.1", 8000, root),
            vec!["manage.py", "runserver", "127.0.0.1:8000", "--noreload"]
        );

        let flask = PythonEntry {
            kind: PythonEntryKind::Flask,
            entry: "app.py".to_string(),
            app_target: Some("app".to_string()),
        };
        assert_eq!(
            build_python_server_args(&flask, "0.0.0.0", 5000, root),
            vec!["-m", "flask", "--app", "app", "run", "--host", "0.0.0.0", "--port", "5000"]
        );

        let fastapi = PythonEntry {
            kind: PythonEntryKind::FastApi,
            entry: "main.py".to_string(),
            app_target: Some("main:app".to_string()),
        };
        assert_eq!(
            build_python_server_args(&fastapi, "127.0.0.1", 9000, root),
            vec!["-m", "uvicorn", "main:app", "--host", "127.0.0.1", "--port", "9000"]
        );

        let http = PythonEntry {
            kind: PythonEntryKind::HttpServer,
            entry: "http.server".to_string(),
            app_target: None,
        };
        assert_eq!(
            build_python_server_args(&http, "127.0.0.1", 8080, root),
            vec![
                "-m",
                "http.server",
                "8080",
                "--bind",
                "127.0.0.1",
                "--directory",
                "/data/workspace/site"
            ]
        );
    }

    #[test]
    fn venv_and_pip_target_args() {
        assert_eq!(
            build_venv_args(Path::new("/data/webforge/venvs/p1")),
            vec!["-m", "venv", "/data/webforge/venvs/p1"]
        );
        assert_eq!(
            build_pip_install_args(
                Path::new("/data/webforge/venvs/p1/site-packages"),
                Path::new("/data/workspace/site/requirements.txt")
            ),
            vec![
                "-m",
                "pip",
                "install",
                "--target",
                "/data/webforge/venvs/p1/site-packages",
                "-r",
                "/data/workspace/site/requirements.txt"
            ]
        );
    }

    #[test]
    fn parse_requirements_names_only() {
        let text = "# komentar\nDjango==5.0\nflask>=3.0 # web\n-r other.txt\nuvicorn[standard]==0.30; python_version>=\"3.9\"\nrequests @ https://example.invalid/requests.whl\n";
        assert_eq!(
            parse_requirements(text),
            vec!["django", "flask", "uvicorn", "requests"]
        );
    }

    #[test]
    fn parse_pip_list_json() {
        let out = r#"[{"name":"Flask","version":"3.0.3"},{"name":"pip","version":"24.2"}]"#;
        assert_eq!(
            parse_pip_list(out),
            vec![
                ("Flask".to_string(), "3.0.3".to_string()),
                ("pip".to_string(), "24.2".to_string())
            ]
        );
        assert!(parse_pip_list("bukan json").is_empty());
    }

    #[test]
    fn find_binary_prefers_bundled_libpython() {
        let dir = fixture(&[("libpython.so", "fake")]);
        assert_eq!(find_python_binary(Some(&dir)), Some(dir.join("libpython.so")));
        assert!(is_bundled_python(&dir.join("libpython.so"), Some(&dir)));
        std::fs::remove_dir_all(dir).ok();
    }
}
