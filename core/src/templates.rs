use crate::util::escape_html;
use std::path::Path;

pub const TEMPLATES: [&str; 5] = ["static", "native-php", "flask", "express", "react-vite"];

pub fn write_template(root: &Path, template: &str, project_name: &str) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let w = |rel: &str, content: &str| -> Result<(), String> {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(p, content).map_err(|e| e.to_string())
    };
    match template {
        "static" => {
            w("index.html", &format!(
                "<!DOCTYPE html>\n<html lang=\"id\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>{name}</title>\n<style>\nbody{{font-family:system-ui,sans-serif;background:#F3F5FA;color:#20283A;display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0}}\n.card{{background:#fff;border-radius:16px;padding:32px;box-shadow:0 4px 24px rgba(0,0,0,.08);text-align:center;max-width:420px}}\nh1{{color:#5576E8}}\n</style>\n</head>\n<body>\n<div class=\"card\">\n<h1>{name}</h1>\n<p>Proyek statis WebForge Mobile berhasil dibuat.</p>\n<p id=\"status\">Server statis berjalan.</p>\n</div>\n<script>document.getElementById('status').textContent='Dimuat pada '+new Date().toLocaleString('id-ID');</script>\n</body>\n</html>\n",
                name = escape_html(project_name)
            ))?;
            w("README.md", &format!("# {}\n\nProyek HTML statis. Jalankan lewat menu Servers di WebForge Mobile (engine static).\n", project_name))?;
        }
        "native-php" => {
            w("index.php", &format!(
                "<?php\n$projectName = {};\n$now = date('Y-m-d H:i:s');\n?>\n<!DOCTYPE html>\n<html lang=\"id\">\n<head><meta charset=\"UTF-8\"><title><?php echo htmlspecialchars($projectName); ?></title></head>\n<body>\n<h1><?php echo htmlspecialchars($projectName); ?></h1>\n<p>PHP berjalan. Waktu server: <?php echo $now; ?></p>\n</body>\n</html>\n",
                serde_json::to_string(project_name).unwrap_or_else(|_| "\"Project\"".to_string())
            ))?;
            w("README.md", &format!("# {}\n\nProyek PHP native.\n\nCatatan jujur: runtime PHP belum dibundel di WebForge Mobile Phase 1, jadi file ini belum bisa dijalankan dari aplikasi sampai runtime PHP tersedia (lihat docs/RUNTIME_PLAN.md).\n", project_name))?;
        }
        "flask" => {
            w("app.py", &format!(
                "from flask import Flask\n\napp = Flask(__name__)\n\n\n@app.route(\"/\")\ndef index():\n    return \"<h1>{}</h1><p>Flask berjalan.</p>\"\n\n\nif __name__ == \"__main__\":\n    app.run(host=\"127.0.0.1\", port=5000)\n",
                escape_html(project_name)
            ))?;
            w("requirements.txt", "Flask==3.0.3\n")?;
            w("README.md", &format!("# {}\n\nProyek Flask.\n\nCatatan jujur: dependensi BELUM terinstal dan runtime Python belum dibundel di WebForge Mobile Phase 1. Untuk menjalankan di lingkungan lain: `pip install -r requirements.txt` lalu `python app.py`.\n", project_name))?;
        }
        "express" => {
            w("package.json", &serde_json::json!({
                "name": crate::util::slugify(project_name),
                "version": "1.0.0",
                "private": true,
                "main": "index.js",
                "scripts": {"start": "node index.js"},
                "dependencies": {"express": "^4.19.2"}
            }).to_string())?;
            w("index.js", &format!(
                "const express = require('express');\nconst app = express();\nconst port = process.env.PORT || 3000;\n\napp.get('/', (req, res) => {{\n  res.send('<h1>{}</h1><p>Express berjalan.</p>');\n}});\n\napp.listen(port, '127.0.0.1', () => {{\n  console.log('Server berjalan di http://127.0.0.1:' + port);\n}});\n",
                escape_html(project_name)
            ))?;
            w("README.md", &format!("# {}\n\nProyek Express.\n\nCatatan jujur: dependensi BELUM terinstal (node_modules belum ada) dan runtime Node.js belum dibundel di WebForge Mobile Phase 1. Untuk menjalankan di lingkungan lain: `npm install` lalu `npm start`.\n", project_name))?;
        }
        "react-vite" => {
            w("package.json", &serde_json::json!({
                "name": crate::util::slugify(project_name),
                "version": "1.0.0",
                "private": true,
                "type": "module",
                "scripts": {"dev": "vite", "build": "vite build", "preview": "vite preview"},
                "dependencies": {"react": "^18.3.1", "react-dom": "^18.3.1"},
                "devDependencies": {"vite": "^5.4.0", "@vitejs/plugin-react": "^4.3.1"}
            }).to_string())?;
            w("index.html", &format!(
                "<!DOCTYPE html>\n<html lang=\"id\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>{}</title>\n</head>\n<body>\n<div id=\"root\"></div>\n<script type=\"module\" src=\"/src/main.jsx\"></script>\n</body>\n</html>\n",
                escape_html(project_name)
            ))?;
            w("src/main.jsx", &format!(
                "import React from 'react';\nimport {{ createRoot }} from 'react-dom/client';\n\nfunction App() {{\n  return (\n    <main style={{{{ fontFamily: 'system-ui, sans-serif', padding: 24 }}}}>\n      <h1>{}</h1>\n      <p>React + Vite siap dikembangkan.</p>\n    </main>\n  );\n}}\n\ncreateRoot(document.getElementById('root')).render(<App />);\n",
                escape_html(project_name)
            ))?;
            w("vite.config.js", "import { defineConfig } from 'vite';\nimport react from '@vitejs/plugin-react';\n\nexport default defineConfig({\n  plugins: [react()],\n});\n")?;
            w("README.md", &format!("# {}\n\nProyek React + Vite minimal.\n\nCatatan jujur: dependensi BELUM terinstal (node_modules belum ada) dan runtime Node.js belum dibundel di WebForge Mobile Phase 1. Untuk menjalankan di lingkungan lain: `npm install` lalu `npm run dev`.\n", project_name))?;
        }
        other => return Err(format!("Template tidak dikenal: {}", other)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_real_template_files() {
        for t in TEMPLATES {
            let dir = std::env::temp_dir().join(format!("wf-tpl-test-{}", uuid::Uuid::new_v4()));
            write_template(&dir, t, "Demo <b>Proj</b>").unwrap();
            let has_main = ["index.html", "index.php", "app.py", "index.js"]
                .iter()
                .any(|f| dir.join(f).exists());
            assert!(has_main, "template {} missing main file", t);
            assert!(dir.join("README.md").exists());
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn unknown_template_errors() {
        let dir = std::env::temp_dir().join(format!("wf-tpl-test-{}", uuid::Uuid::new_v4()));
        assert!(write_template(&dir, "laravel-full", "X").is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
