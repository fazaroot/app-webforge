use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct FrameworkInfo {
    pub framework: String,
    pub language: String,
    pub version: Option<String>,
    pub recommended_runtime: String,
    pub recommended_document_root: String,
    pub readiness: String,
    pub notes: Vec<String>,
    pub detected_files: Vec<String>,
}

fn exists(root: &Path, rel: &str) -> bool {
    root.join(rel).exists()
}

fn read_text(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

fn composer_deps(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(text) = read_text(root, "composer.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            for section in ["require", "require-dev"] {
                if let Some(obj) = v.get(section).and_then(|x| x.as_object()) {
                    for (k, val) in obj {
                        if let Some(s) = val.as_str() {
                            map.insert(k.to_lowercase(), s.to_string());
                        }
                    }
                }
            }
        }
    }
    map
}

fn package_deps(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(text) = read_text(root, "package.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            for section in ["dependencies", "devDependencies"] {
                if let Some(obj) = v.get(section).and_then(|x| x.as_object()) {
                    for (k, val) in obj {
                        if let Some(s) = val.as_str() {
                            map.insert(k.to_lowercase(), s.to_string());
                        }
                    }
                }
            }
        }
    }
    map
}

fn python_deps(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(text) = read_text(root, "requirements.txt") {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let name_end = line
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                .unwrap_or(line.len());
            let name = line[..name_end].to_lowercase().replace('_', "-");
            let ver = line[name_end..].trim_start_matches(['=', '>', '<', '~', '!', ' ']).to_string();
            if !name.is_empty() {
                map.insert(name, ver);
            }
        }
    }
    if let Some(text) = read_text(root, "pyproject.toml") {
        // Minimal scan: find quoted dependency strings like "django>=5.0"
        for line in text.lines() {
            let t = line.trim();
            if let Some(start) = t.find('"') {
                if let Some(end_rel) = t[start + 1..].find('"') {
                    let inner = &t[start + 1..start + 1 + end_rel];
                    let name_end = inner
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                        .unwrap_or(inner.len());
                    let name = inner[..name_end].to_lowercase().replace('_', "-");
                    if ["django", "flask", "fastapi", "pyramid", "uvicorn"].contains(&name.as_str()) {
                        let ver = inner[name_end..].trim_start_matches(['=', '>', '<', '~', '!', ' ']).to_string();
                        map.insert(name, ver);
                    }
                }
            }
        }
    }
    map
}

fn python_files_contain(root: &Path, needle: &str) -> bool {
    for name in ["app.py", "main.py", "wsgi.py", "server.py", "run.py"] {
        if let Some(text) = read_text(root, name) {
            if text.contains(needle) {
                return true;
            }
        }
    }
    false
}

fn has_php_file(root: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().map(|x| x == "php").unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

fn has_html_file(root: &Path) -> bool {
    exists(root, "index.html")
        || std::fs::read_dir(root)
            .map(|rd| {
                rd.flatten().any(|e| {
                    e.path().is_file() && e.path().extension().map(|x| x == "html").unwrap_or(false)
                })
            })
            .unwrap_or(false)
}

fn clean_version(v: Option<&String>) -> Option<String> {
    v.map(|s| s.trim_start_matches(['^', '~', '>', '<', '=', ' ']).to_string())
        .filter(|s| !s.is_empty())
}

/// Detect framework. `available_runtimes` contains runtime ids known available
/// (e.g. "static", "php", "python", "node") used to compute readiness honestly.
pub fn detect(root: &Path, available_runtimes: &HashSet<String>) -> FrameworkInfo {
    let mut detected: Vec<String> = Vec::new();
    let note = |detected: &mut Vec<String>, rel: &str| {
        if exists(root, rel) && !detected.iter().any(|d| d == rel) {
            detected.push(rel.to_string());
        }
    };
    for f in [
        "artisan", "spark", "wp-config.php", "wp-content", "bin/console", "composer.json",
        "package.json", "manage.py", "requirements.txt", "pyproject.toml", "index.php",
        "index.html", "app.py", "main.py", "vite.config.js", "vite.config.ts",
        "next.config.js", "next.config.mjs", "next.config.ts", "nuxt.config.ts", "nuxt.config.js",
        "system/core/CodeIgniter.php", "application/config/config.php", "config/bundles.php",
    ] {
        note(&mut detected, f);
    }

    let composer = composer_deps(root);
    let node = package_deps(root);
    let py = python_deps(root);

    let make = |framework: &str,
                language: &str,
                version: Option<String>,
                runtime: &str,
                docroot: &str,
                notes: Vec<String>,
                detected: Vec<String>|
     -> FrameworkInfo {
        let readiness = if runtime == "static" {
            "ready".to_string()
        } else if !available_runtimes.contains(runtime) {
            "needs_runtime".to_string()
        } else {
            // runtime present: check dependency dirs
            let deps_ok = match runtime {
                "php" => !exists(root, "composer.json") || exists(root, "vendor"),
                "node" => !exists(root, "package.json") || exists(root, "node_modules"),
                "python" => exists(root, ".venv") || exists(root, "venv") || (!exists(root, "requirements.txt") && !exists(root, "pyproject.toml")),
                _ => true,
            };
            if deps_ok { "ready".to_string() } else { "needs_dependencies".to_string() }
        };
        let mut notes = notes;
        if readiness == "needs_runtime" {
            notes.push(format!("Runtime {} belum tersedia di Phase 1", runtime));
        } else if readiness == "needs_dependencies" {
            notes.push("Dependensi belum terinstal (vendor/node_modules/.venv tidak ditemukan)".to_string());
        }
        FrameworkInfo {
            framework: framework.to_string(),
            language: language.to_string(),
            version,
            recommended_runtime: runtime.to_string(),
            recommended_document_root: docroot.to_string(),
            readiness,
            notes,
            detected_files: detected,
        }
    };

    // --- PHP frameworks ---
    if exists(root, "artisan") || composer.contains_key("laravel/framework") {
        return make(
            "Laravel", "PHP", clean_version(composer.get("laravel/framework")), "php", "public",
            vec!["Document root Laravel adalah folder public".to_string()], detected,
        );
    }
    if exists(root, "spark") || composer.contains_key("codeigniter4/framework") {
        return make(
            "CodeIgniter 4", "PHP", clean_version(composer.get("codeigniter4/framework")), "php", "public",
            vec!["Document root CodeIgniter 4 adalah folder public".to_string()], detected,
        );
    }
    if exists(root, "system/core/CodeIgniter.php") || composer.contains_key("codeigniter/framework") {
        return make(
            "CodeIgniter 3", "PHP", clean_version(composer.get("codeigniter/framework")), "php", ".",
            vec![], detected,
        );
    }
    if exists(root, "wp-config.php") || exists(root, "wp-content") || exists(root, "wp-includes") {
        return make("WordPress", "PHP", None, "php", ".", vec!["WordPress membutuhkan database MySQL/MariaDB".to_string()], detected);
    }
    if composer.contains_key("symfony/framework-bundle") || (exists(root, "bin/console") && exists(root, "config/bundles.php")) {
        return make(
            "Symfony", "PHP", clean_version(composer.get("symfony/framework-bundle")), "php", "public",
            vec![], detected,
        );
    }
    if composer.contains_key("slim/slim") {
        let docroot = if exists(root, "public") { "public" } else { "." };
        return make("Slim", "PHP", clean_version(composer.get("slim/slim")), "php", docroot, vec![], detected);
    }

    // --- Python frameworks ---
    if exists(root, "manage.py") || py.contains_key("django") {
        return make(
            "Django", "Python", clean_version(py.get("django")), "python", ".",
            vec!["Jalankan dengan manage.py runserver setelah dependensi terinstal".to_string()], detected,
        );
    }
    if py.contains_key("fastapi") || python_files_contain(root, "FastAPI(") {
        return make("FastAPI", "Python", clean_version(py.get("fastapi")), "python", ".", vec![], detected);
    }
    if py.contains_key("flask") || python_files_contain(root, "Flask(") {
        return make("Flask", "Python", clean_version(py.get("flask")), "python", ".", vec![], detected);
    }
    if py.contains_key("pyramid") || python_files_contain(root, "Configurator") {
        return make("Pyramid", "Python", clean_version(py.get("pyramid")), "python", ".", vec![], detected);
    }

    // --- Node frameworks ---
    if node.contains_key("next") || exists(root, "next.config.js") || exists(root, "next.config.mjs") || exists(root, "next.config.ts") {
        return make("Next.js", "Node.js", clean_version(node.get("next")), "node", ".", vec![], detected);
    }
    if node.contains_key("nuxt") || exists(root, "nuxt.config.ts") || exists(root, "nuxt.config.js") {
        return make("Nuxt", "Node.js", clean_version(node.get("nuxt")), "node", ".", vec![], detected);
    }
    if node.contains_key("express") {
        return make("Express", "Node.js", clean_version(node.get("express")), "node", ".", vec![], detected);
    }
    if node.contains_key("react") {
        let docroot = if exists(root, "dist") { "dist" } else { "." };
        let mut notes = vec![];
        if node.contains_key("vite") {
            notes.push("Proyek React dengan Vite; hasil build ada di dist".to_string());
        }
        return make("React", "Node.js", clean_version(node.get("react")), "node", docroot, notes, detected);
    }
    if node.contains_key("vue") {
        let docroot = if exists(root, "dist") { "dist" } else { "." };
        return make("Vue", "Node.js", clean_version(node.get("vue")), "node", docroot, vec![], detected);
    }
    if node.contains_key("vite") || exists(root, "vite.config.js") || exists(root, "vite.config.ts") {
        let docroot = if exists(root, "dist") { "dist" } else { "." };
        return make("Vite", "Node.js", clean_version(node.get("vite")), "node", docroot, vec![], detected);
    }
    if exists(root, "package.json") {
        return make("Unknown", "Node.js", None, "node", ".", vec!["package.json ditemukan tetapi framework tidak dikenali".to_string()], detected);
    }

    // --- Plain PHP / static ---
    if has_php_file(root) {
        return make("Native PHP", "PHP", None, "php", ".", vec![], detected);
    }
    if has_html_file(root) {
        return make("Static HTML", "Static", None, "static", ".", vec![], detected);
    }

    FrameworkInfo {
        framework: "Unknown".to_string(),
        language: "Unknown".to_string(),
        version: None,
        recommended_runtime: "static".to_string(),
        recommended_document_root: ".".to_string(),
        readiness: "unknown".to_string(),
        notes: vec!["Tidak ada file penanda framework yang dikenali".to_string()],
        detected_files: detected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wf-fw-test-{}", uuid::Uuid::new_v4()));
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

    fn avail(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_laravel() {
        let d = fixture(&[
            ("artisan", "#!/usr/bin/env php"),
            ("composer.json", r#"{"require":{"laravel/framework":"^11.0"}}"#),
        ]);
        let f = detect(&d, &avail(&["static"]));
        assert_eq!(f.framework, "Laravel");
        assert_eq!(f.language, "PHP");
        assert_eq!(f.version.as_deref(), Some("11.0"));
        assert_eq!(f.recommended_document_root, "public");
        assert_eq!(f.readiness, "needs_runtime");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn detects_codeigniter4_and_3() {
        let d = fixture(&[("spark", ""), ("composer.json", r#"{"require":{"codeigniter4/framework":"^4.5"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static", "php"])).framework, "CodeIgniter 4");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("system/core/CodeIgniter.php", "<?php")]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "CodeIgniter 3");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn detects_wordpress_symfony_slim() {
        let d = fixture(&[("wp-config.php", "<?php")]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "WordPress");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("composer.json", r#"{"require":{"symfony/framework-bundle":"^7.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Symfony");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("composer.json", r#"{"require":{"slim/slim":"^4.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Slim");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn detects_native_php_and_static() {
        let d = fixture(&[("index.php", "<?php echo 1;")]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Native PHP");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("index.html", "<html></html>")]);
        let f = detect(&d, &avail(&["static"]));
        assert_eq!(f.framework, "Static HTML");
        assert_eq!(f.readiness, "ready");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn detects_python_frameworks() {
        let d = fixture(&[("manage.py", ""), ("requirements.txt", "Django==5.0\n")]);
        let f = detect(&d, &avail(&["static"]));
        assert_eq!(f.framework, "Django");
        assert_eq!(f.version.as_deref(), Some("5.0"));
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("app.py", "from flask import Flask\napp = Flask(__name__)")]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Flask");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("main.py", "from fastapi import FastAPI\napp = FastAPI()")]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "FastAPI");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("requirements.txt", "pyramid==2.0\n")]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Pyramid");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn detects_node_frameworks() {
        let d = fixture(&[("package.json", r#"{"dependencies":{"express":"^4.19.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Express");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("package.json", r#"{"dependencies":{"next":"14.2.0","react":"18.3.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Next.js");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("package.json", r#"{"dependencies":{"nuxt":"^3.12.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Nuxt");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("package.json", r#"{"dependencies":{"react":"18.3.0"},"devDependencies":{"vite":"^5.0.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "React");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("package.json", r#"{"dependencies":{"vue":"^3.4.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Vue");
        std::fs::remove_dir_all(&d).ok();
        let d = fixture(&[("package.json", r#"{"devDependencies":{"vite":"^5.0.0"}}"#)]);
        assert_eq!(detect(&d, &avail(&["static"])).framework, "Vite");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn unknown_for_empty_dir() {
        let d = fixture(&[("readme.txt", "hello")]);
        let f = detect(&d, &avail(&["static"]));
        assert_eq!(f.framework, "Unknown");
        assert_eq!(f.readiness, "unknown");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn readiness_needs_dependencies_when_runtime_present() {
        let d = fixture(&[("package.json", r#"{"dependencies":{"express":"^4.0.0"}}"#)]);
        let f = detect(&d, &avail(&["static", "node"]));
        assert_eq!(f.readiness, "needs_dependencies");
        std::fs::remove_dir_all(d).ok();
    }
}
