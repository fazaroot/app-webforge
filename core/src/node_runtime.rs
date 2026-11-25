//! Node.js runtime: inspeksi package.json dan pemilihan entry yang akan
//! dijalankan NodeService lewat node::Start.
//!
//! NodeService menjalankan Node tanpa shell, jadi script npm yang
//! mengandung operator shell tidak bisa dieksekusi apa adanya. Modul ini
//! menerjemahkan pola umum (node <file>, vite, bin lokal) menjadi argumen
//! Node nyata dan mengembalikan Err jujur untuk script yang membutuhkan
//! shell/npm CLI penuh.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeProjectInfo {
    pub scripts: BTreeMap<String, String>,
    /// Nilai field "main" package.json bila ada.
    pub entry: Option<String>,
    pub has_node_modules: bool,
    /// "npm" | "yarn" | "pnpm" | "bun" dari packageManager/lockfile.
    pub package_manager: Option<String>,
}

fn read_package_json(project_path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(project_path.join("package.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()
}

fn detect_package_manager(project_path: &Path, package_json: Option<&serde_json::Value>) -> Option<String> {
    if let Some(pm) = package_json
        .and_then(|v| v.get("packageManager"))
        .and_then(|v| v.as_str())
    {
        let name = pm.split('@').next().unwrap_or("").trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    if project_path.join("package-lock.json").is_file() {
        return Some("npm".to_string());
    }
    if project_path.join("yarn.lock").is_file() {
        return Some("yarn".to_string());
    }
    if project_path.join("pnpm-lock.yaml").is_file() {
        return Some("pnpm".to_string());
    }
    if project_path.join("bun.lockb").is_file() || project_path.join("bun.lock").is_file() {
        return Some("bun".to_string());
    }
    None
}

/// Inspeksi proyek Node dari package.json dan lockfile nyata. Proyek tanpa
/// package.json menghasilkan scripts kosong dan entry None; pemilihan
/// fallback file dilakukan choose_node_entry.
pub fn inspect_node_project(project_path: &Path) -> NodeProjectInfo {
    let package_json = read_package_json(project_path);
    let mut scripts = BTreeMap::new();
    let mut entry = None;
    if let Some(value) = package_json.as_ref() {
        if let Some(obj) = value.get("scripts").and_then(|v| v.as_object()) {
            for (name, script) in obj {
                if let Some(script) = script.as_str() {
                    scripts.insert(name.clone(), script.to_string());
                }
            }
        }
        if let Some(main) = value.get("main").and_then(|v| v.as_str()) {
            let main = main.trim();
            if !main.is_empty() {
                entry = Some(main.to_string());
            }
        }
    }
    NodeProjectInfo {
        scripts,
        entry,
        has_node_modules: project_path.join("node_modules").is_dir(),
        package_manager: detect_package_manager(project_path, package_json.as_ref()),
    }
}

fn tokenize_script(script: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in script.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(c);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn has_shell_operator(script: &str) -> bool {
    ["&&", "||", ";", "|", ">", "<", "`", "$("]
        .iter()
        .any(|op| script.contains(op))
}

fn strip_env_assignments(tokens: Vec<String>) -> Vec<String> {
    let mut idx = 0;
    if tokens.get(idx).map(|s| s.as_str()) == Some("cross-env") {
        idx += 1;
    }
    while idx < tokens.len() {
        let token = &tokens[idx];
        let is_assignment = token
            .split_once('=')
            .map(|(name, _)| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !name.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            })
            .unwrap_or(false);
        if is_assignment {
            idx += 1;
        } else {
            break;
        }
    }
    tokens.into_iter().skip(idx).collect()
}

fn vite_args(tokens_after_vite: &[String], host: &str, port: u16) -> Vec<String> {
    let mut args = vec!["node_modules/vite/bin/vite.js".to_string()];
    let mut i = 0;
    while i < tokens_after_vite.len() {
        let token = &tokens_after_vite[i];
        if token == "--host" || token == "--port" {
            i += 2;
            continue;
        }
        if token.starts_with("--host=") || token.starts_with("--port=") {
            i += 1;
            continue;
        }
        if i == 0 && token == "dev" {
            i += 1;
            continue;
        }
        args.push(token.clone());
        i += 1;
    }
    args.push("--host".to_string());
    args.push(host.to_string());
    args.push("--port".to_string());
    args.push(port.to_string());
    args
}

fn script_to_node_args(script: &str, host: &str, port: u16) -> Result<Vec<String>, String> {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return Err("Script Node kosong".to_string());
    }
    if has_shell_operator(trimmed) {
        return Err(
            "Script Node mengandung operator shell; NodeService menjalankan Node tanpa shell, jadi script ini harus dijalankan lewat npm CLI/shell oleh koordinator"
                .to_string(),
        );
    }
    let tokens = strip_env_assignments(tokenize_script(trimmed));
    if tokens.is_empty() {
        return Err("Script Node tidak menghasilkan perintah".to_string());
    }
    if tokens[0] == "node" {
        if tokens.len() < 2 {
            return Err("Script Node hanya berisi 'node' tanpa file entry".to_string());
        }
        return Ok(tokens.into_iter().skip(1).collect());
    }
    if let Some(vite_idx) = tokens.iter().position(|t| t == "vite") {
        return Ok(vite_args(&tokens[vite_idx + 1..], host, port));
    }
    if tokens[0] == "next" && tokens.get(1).map(|s| s.as_str()) == Some("start") {
        let mut args = vec!["node_modules/.bin/next".to_string(), "start".to_string()];
        let rest = &tokens[2..];
        let has_port = rest
            .iter()
            .any(|t| t == "-p" || t == "--port" || t.starts_with("--port="));
        let has_host = rest
            .iter()
            .any(|t| t == "-H" || t == "--hostname" || t.starts_with("--hostname="));
        args.extend(rest.iter().cloned());
        if !has_host {
            args.push("-H".to_string());
            args.push(host.to_string());
        }
        if !has_port {
            args.push("-p".to_string());
            args.push(port.to_string());
        }
        return Ok(args);
    }
    // Bin lokal lain (mis. nodemon, tsx) diterjemahkan ke node_modules/.bin.
    // Bila bin itu bukan JavaScript atau belum terinstal, Node akan gagal
    // nyata dan log NodeService menunjukkannya.
    let mut args = vec![format!("node_modules/.bin/{}", tokens[0])];
    args.extend(tokens.into_iter().skip(1));
    Ok(args)
}

fn explicit_command_args(
    info: &NodeProjectInfo,
    project_path: &Path,
    command: &str,
    host: &str,
    port: u16,
) -> Result<Vec<String>, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Command Node kosong".to_string());
    }
    if let Some(script) = info.scripts.get(command) {
        return script_to_node_args(script, host, port);
    }
    let looks_like_file = command.ends_with(".js")
        || command.ends_with(".mjs")
        || command.ends_with(".cjs")
        || command.contains('/');
    if looks_like_file {
        if project_path.join(command).is_file() {
            return Ok(vec![command.to_string()]);
        }
        return Err(format!("File entry Node tidak ditemukan: {}", command));
    }
    if command.contains(char::is_whitespace) || command == "node" || command == "vite" {
        return script_to_node_args(command, host, port);
    }
    Err(format!(
        "Command Node '{}' bukan nama script package.json dan bukan file .js/.mjs/.cjs",
        command
    ))
}

/// Pilih argumen Node (tanpa argv[0] "node") untuk NodeService.
/// Prioritas: command eksplisit > scripts.start > scripts.dev > main >
/// server.js/index.js/app.js. Script Vite dev/start mendapat --host dan
/// --port dari server WebForge.
pub fn choose_node_entry(
    info: &NodeProjectInfo,
    project_path: &Path,
    command: Option<&str>,
    host: &str,
    port: u16,
) -> Result<Vec<String>, String> {
    if let Some(command) = command {
        if !command.trim().is_empty() {
            return explicit_command_args(info, project_path, command, host, port);
        }
    }
    if let Some(script) = info.scripts.get("start") {
        return script_to_node_args(script, host, port);
    }
    if let Some(script) = info.scripts.get("dev") {
        return script_to_node_args(script, host, port);
    }
    if let Some(entry) = info.entry.as_ref() {
        if project_path.join(entry).is_file() {
            return Ok(vec![entry.clone()]);
        }
        return Err(format!(
            "File main package.json tidak ditemukan: {}",
            entry
        ));
    }
    for file in ["server.js", "index.js", "app.js"] {
        if project_path.join(file).is_file() {
            return Ok(vec![file.to_string()]);
        }
    }
    Err(
        "Entry Node tidak ditemukan: tidak ada command, scripts.start/dev, main, server.js, index.js, atau app.js"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wf-node-test-{}", crate::util::new_id()));
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
    fn inspect_package_json_scripts_main_and_manager() {
        let d = fixture(&[
            (
                "package.json",
                r#"{"main":"server.js","packageManager":"npm@10.8.2","scripts":{"start":"node server.js","dev":"vite"}}"#,
            ),
            ("server.js", "console.log('ok')"),
            ("node_modules/.keep", ""),
        ]);
        let info = inspect_node_project(&d);
        assert_eq!(info.entry.as_deref(), Some("server.js"));
        assert_eq!(info.package_manager.as_deref(), Some("npm"));
        assert!(info.has_node_modules);
        assert_eq!(
            info.scripts.get("start").map(|s| s.as_str()),
            Some("node server.js")
        );
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn choose_prefers_start_script_over_dev_and_main() {
        let d = fixture(&[
            (
                "package.json",
                r#"{"main":"index.js","scripts":{"start":"node server.js","dev":"vite"}}"#,
            ),
            ("index.js", ""),
            ("server.js", ""),
        ]);
        let info = inspect_node_project(&d);
        assert_eq!(
            choose_node_entry(&info, &d, None, "127.0.0.1", 3000).unwrap(),
            vec!["server.js"]
        );
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn choose_vite_dev_adds_host_and_port() {
        let d = fixture(&[(
            "package.json",
            r#"{"scripts":{"dev":"vite --open"}}"#,
        )]);
        let info = inspect_node_project(&d);
        assert_eq!(
            choose_node_entry(&info, &d, None, "0.0.0.0", 5173).unwrap(),
            vec![
                "node_modules/vite/bin/vite.js",
                "--open",
                "--host",
                "0.0.0.0",
                "--port",
                "5173"
            ]
        );
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn explicit_command_script_name_and_file() {
        let d = fixture(&[
            (
                "package.json",
                r#"{"scripts":{"serve":"node app.js"}}"#,
            ),
            ("app.js", ""),
            ("server.js", ""),
        ]);
        let info = inspect_node_project(&d);
        assert_eq!(
            choose_node_entry(&info, &d, Some("serve"), "127.0.0.1", 3000).unwrap(),
            vec!["app.js"]
        );
        assert_eq!(
            choose_node_entry(&info, &d, Some("server.js"), "127.0.0.1", 3000).unwrap(),
            vec!["server.js"]
        );
        assert!(choose_node_entry(&info, &d, Some("missing.js"), "127.0.0.1", 3000).is_err());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn fallback_files_and_missing_entry() {
        let d = fixture(&[("index.js", "")]);
        let info = inspect_node_project(&d);
        assert_eq!(
            choose_node_entry(&info, &d, None, "127.0.0.1", 3000).unwrap(),
            vec!["index.js"]
        );
        std::fs::remove_dir_all(&d).ok();

        let d = fixture(&[("readme.txt", "halo")]);
        let info = inspect_node_project(&d);
        assert!(choose_node_entry(&info, &d, None, "127.0.0.1", 3000).is_err());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn shell_script_is_rejected_honestly() {
        let d = fixture(&[(
            "package.json",
            r#"{"scripts":{"start":"npm run build && node server.js"}}"#,
        )]);
        let info = inspect_node_project(&d);
        let err = choose_node_entry(&info, &d, None, "127.0.0.1", 3000).unwrap_err();
        assert!(err.contains("operator shell"));
        std::fs::remove_dir_all(d).ok();
    }
}
