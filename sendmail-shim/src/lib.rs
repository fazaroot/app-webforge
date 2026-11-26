//! Shim sendmail WebForge Mobile.
//!
//! Dipakai PHP lewat `sendmail_path`: menerima argumen ala sendmail
//! (`-t`, `-f pengirim`, `-i`, penerima eksplisit) dan pesan RFC822 di
//! stdin, lalu mengirimnya ke server SMTP dari berkas konfigurasi JSON
//! (`--config <path>` atau env `WEBFORGE_SMTP_CONFIG`).
//!
//! Kontrak kegagalan: konfigurasi tidak ada/tidak valid atau SMTP
//! gagal -> exit nonzero dengan pesan jelas di stderr. Shim tidak
//! pernah berpura-pura berhasil.

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use serde::Deserialize;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    None,
    Starttls,
    Tls,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_security")]
    pub security: SmtpSecurity,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

fn default_security() -> SmtpSecurity {
    SmtpSecurity::Starttls
}

impl SmtpConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("Konfigurasi SMTP tidak valid: host kosong".to_string());
        }
        if self.port == 0 {
            return Err("Konfigurasi SMTP tidak valid: port harus antara 1 dan 65535".to_string());
        }
        if extract_addresses(&self.from).is_empty() {
            return Err(format!(
                "Konfigurasi SMTP tidak valid: alamat from '{}' tidak valid",
                self.from
            ));
        }
        Ok(())
    }
}

/// Muat konfigurasi dari path eksplisit, atau env WEBFORGE_SMTP_CONFIG.
pub fn load_config(explicit: Option<&str>) -> Result<SmtpConfig, String> {
    let path = match explicit {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => match std::env::var("WEBFORGE_SMTP_CONFIG") {
            Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => {
                return Err(
                    "Konfigurasi SMTP tidak ditemukan: berikan --config <path> atau set env WEBFORGE_SMTP_CONFIG (SMTP mungkin belum dikonfigurasi di WebForge)"
                        .to_string(),
                )
            }
        },
    };
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "Konfigurasi SMTP tidak dapat dibaca dari {}: {} (SMTP mungkin belum dikonfigurasi di WebForge)",
            path.display(),
            e
        )
    })?;
    let cfg: SmtpConfig = serde_json::from_str(&text).map_err(|e| {
        format!("Konfigurasi SMTP di {} tidak valid: {}", path.display(), e)
    })?;
    cfg.validate()?;
    Ok(cfg)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendmailArgs {
    pub config: Option<String>,
    pub read_headers: bool,
    pub from: Option<String>,
    pub recipients: Vec<String>,
}

/// Parse argumen ala sendmail. Flag yang dikenal: --config, -t, -i,
/// -f. Flag lain yang diawali '-' diabaikan (kompatibilitas sendmail),
/// kecuali -f yang mengambil nilai berikutnya.
pub fn parse_args(args: &[String]) -> Result<SendmailArgs, String> {
    let mut out = SendmailArgs::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(path) = arg.strip_prefix("--config=") {
            out.config = Some(path.to_string());
        } else if arg == "--config" {
            i += 1;
            let path = args
                .get(i)
                .ok_or_else(|| "Argumen --config membutuhkan path".to_string())?;
            out.config = Some(path.clone());
        } else if arg == "-t" {
            out.read_headers = true;
        } else if arg == "-i" {
            // Abaikan titik tunggal di awal baris (perilaku bawaan kami).
        } else if arg == "-f" {
            i += 1;
            let from = args
                .get(i)
                .ok_or_else(|| "Argumen -f membutuhkan alamat pengirim".to_string())?;
            out.from = Some(from.clone());
        } else if let Some(from) = arg.strip_prefix("-f") {
            if !from.is_empty() {
                out.from = Some(from.to_string());
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            // Flag sendmail lain (-F, -o, -O, ...) diabaikan; -F/-o/-O
            // yang berdiri sendiri mengambil nilai berikutnya.
            if matches!(arg.as_str(), "-F" | "-o" | "-O") {
                i += 1;
            }
        } else if !arg.trim().is_empty() {
            out.recipients.push(arg.clone());
        }
        i += 1;
    }
    Ok(out)
}

/// Ekstrak alamat email dari nilai header ("Nama <a@b>, c@d").
pub fn extract_addresses(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_angle = false;
    let mut in_quote = false;
    let mut parts: Vec<String> = Vec::new();
    for c in value.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                current.push(c);
            }
            '<' if !in_quote => {
                in_angle = true;
                current.push(c);
            }
            '>' if !in_quote => {
                in_angle = false;
                current.push(c);
            }
            ',' if !in_quote && !in_angle => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    for part in parts {
        let part = part.trim();
        let addr = if let (Some(start), Some(end)) = (part.find('<'), part.rfind('>')) {
            if end > start {
                part[start + 1..end].trim().to_string()
            } else {
                part.to_string()
            }
        } else {
            part
                .split_whitespace()
                .find(|tok| tok.contains('@'))
                .unwrap_or(part)
                .trim_matches(|c| c == '<' || c == '>' || c == '"' || c == ';')
                .to_string()
        };
        if addr.contains('@') && !addr.contains(char::is_whitespace) {
            out.push(addr);
        }
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedMessage {
    pub from: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    /// Pesan mentah tanpa header Bcc (sendmail membuang Bcc sebelum kirim).
    pub raw_without_bcc: Vec<u8>,
}

/// Parse header RFC822 dari pesan mentah (dengan unfolding baris
/// lanjutan) dan hasilkan salinan tanpa header Bcc.
pub fn parse_message(raw: &[u8]) -> ParsedMessage {
    let text = String::from_utf8_lossy(raw);
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut body_start = text.len();
    let mut offset = 0usize;
    let mut current: Option<(String, String)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            body_start = offset + line.len();
            break;
        }
        if (trimmed.starts_with(' ') || trimmed.starts_with('\t')) && current.is_some() {
            let (_, value) = current.as_mut().unwrap();
            value.push(' ');
            value.push_str(trimmed.trim());
        } else if let Some((name, value)) = trimmed.split_once(':') {
            if let Some(done) = current.take() {
                headers.push(done);
            }
            current = Some((name.trim().to_string(), value.trim().to_string()));
        } else {
            // Baris bukan header: anggap awal body.
            body_start = offset;
            break;
        }
        offset += line.len();
    }
    if let Some(done) = current.take() {
        headers.push(done);
    }

    let mut parsed = ParsedMessage::default();
    let mut kept: Vec<String> = Vec::new();
    for (name, value) in &headers {
        match name.to_ascii_lowercase().as_str() {
            "from" => {
                parsed.from = extract_addresses(value).into_iter().next();
                kept.push(format!("From: {}", value));
            }
            "to" => {
                parsed.to.extend(extract_addresses(value));
                kept.push(format!("To: {}", value));
            }
            "cc" => {
                parsed.cc.extend(extract_addresses(value));
                kept.push(format!("Cc: {}", value));
            }
            "bcc" => {
                parsed.bcc.extend(extract_addresses(value));
                // Bcc tidak ikut dikirim dalam isi pesan.
            }
            _ => {
                // Pertahankan nama header asli sebisa mungkin: huruf
                // pertama kapital untuk header umum tidak penting bagi
                // SMTP; nama apa adanya cukup.
                kept.push(format!("{}: {}", name, value));
            }
        }
    }
    let body = &text[body_start.min(text.len())..];
    let mut out = String::new();
    for line in &kept {
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.push_str(body);
    parsed.raw_without_bcc = out.into_bytes();
    parsed
}

/// Tentukan envelope: pengirim dari -f, lalu header From, lalu
/// config.from. Penerima dari argumen, atau To/Cc/Bcc bila -t.
pub fn build_envelope(
    args: &SendmailArgs,
    parsed: &ParsedMessage,
    cfg: &SmtpConfig,
) -> Result<(Address, Vec<Address>), String> {
    let from_raw = args
        .from
        .clone()
        .or_else(|| parsed.from.clone())
        .or_else(|| extract_addresses(&cfg.from).into_iter().next())
        .ok_or_else(|| "Alamat pengirim tidak ditemukan (-f, header From, dan config.from kosong)".to_string())?;
    let from = Address::from_str(&from_raw)
        .map_err(|_| format!("Alamat pengirim '{}' tidak valid", from_raw))?;

    let mut recipient_raw: Vec<String> = args.recipients.clone();
    if args.read_headers {
        recipient_raw.extend(parsed.to.iter().cloned());
        recipient_raw.extend(parsed.cc.iter().cloned());
        recipient_raw.extend(parsed.bcc.iter().cloned());
    }
    let mut recipients: Vec<Address> = Vec::new();
    for r in &recipient_raw {
        for addr in extract_addresses(r) {
            let parsed_addr = Address::from_str(&addr)
                .map_err(|_| format!("Alamat penerima '{}' tidak valid", addr))?;
            if !recipients.contains(&parsed_addr) {
                recipients.push(parsed_addr);
            }
        }
    }
    if recipients.is_empty() {
        return Err(
            "Tidak ada penerima: berikan penerima sebagai argumen atau gunakan -t dengan header To/Cc/Bcc"
                .to_string(),
        );
    }
    Ok((from, recipients))
}

/// Kirim pesan mentah lewat SMTP sesuai konfigurasi.
pub fn send_raw(cfg: &SmtpConfig, from: Address, recipients: Vec<Address>, raw: &[u8]) -> Result<(), String> {
    let envelope = Envelope::new(Some(from), recipients)
        .map_err(|e| format!("Envelope email tidak valid: {}", e))?;
    let mut builder = match cfg.security {
        SmtpSecurity::Tls => SmtpTransport::relay(&cfg.host)
            .map_err(|e| format!("Gagal menyiapkan koneksi SMTP TLS ke {}: {}", cfg.host, e))?,
        SmtpSecurity::Starttls => SmtpTransport::starttls_relay(&cfg.host)
            .map_err(|e| format!("Gagal menyiapkan koneksi SMTP STARTTLS ke {}: {}", cfg.host, e))?,
        SmtpSecurity::None => SmtpTransport::builder_dangerous(&cfg.host),
    };
    builder = builder.port(cfg.port).timeout(Some(Duration::from_secs(30)));
    if let (Some(user), Some(password)) = (&cfg.username, &cfg.password) {
        if !user.trim().is_empty() {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
    }
    let transport = builder.build();
    transport
        .send_raw(&envelope, raw)
        .map_err(|e| format!("Pengiriman SMTP ke {}:{} gagal: {}", cfg.host, cfg.port, e))?;
    Ok(())
}

/// Jalankan shim: parse argumen, baca stdin, kirim. Mengembalikan
/// pesan sukses untuk stdout.
pub fn run(args: &[String], stdin: &[u8]) -> Result<String, String> {
    let parsed_args = parse_args(args)?;
    let cfg = load_config(parsed_args.config.as_deref())?;
    if stdin.is_empty() {
        return Err("Pesan kosong: tidak ada data RFC822 di stdin".to_string());
    }
    let parsed = parse_message(stdin);
    let (from, recipients) = build_envelope(&parsed_args, &parsed, &cfg)?;
    let count = recipients.len();
    send_raw(&cfg, from, recipients, &parsed.raw_without_bcc)?;
    Ok(format!(
        "Pesan terkirim ke {} penerima lewat {}:{}",
        count, cfg.host, cfg.port
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_sendmail_style() {
        let args: Vec<String> = ["--config", "/data/mail/smtp.json", "-t", "-i", "-f", "a@b.id"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.config.as_deref(), Some("/data/mail/smtp.json"));
        assert!(parsed.read_headers);
        assert_eq!(parsed.from.as_deref(), Some("a@b.id"));
        assert!(parsed.recipients.is_empty());

        let args: Vec<String> = ["-fa@b.id", "x@y.id", "z@w.id"].iter().map(|s| s.to_string()).collect();
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.from.as_deref(), Some("a@b.id"));
        assert_eq!(parsed.recipients, vec!["x@y.id", "z@w.id"]);
        assert!(!parsed.read_headers);
    }

    #[test]
    fn extract_addresses_variants() {
        assert_eq!(
            extract_addresses("Budi <budi@example.com>, siti@example.com"),
            vec!["budi@example.com", "siti@example.com"]
        );
        assert_eq!(extract_addresses("a@b.id"), vec!["a@b.id"]);
        assert!(extract_addresses("bukan alamat").is_empty());
    }

    #[test]
    fn parse_message_headers_and_bcc_stripped() {
        let raw = b"From: Pengirim <from@example.com>\r\nTo: to@example.com\r\nCc: cc@example.com\r\nBcc: bcc@example.com\r\nSubject: Halo\r\n\r\nIsi pesan.\r\n";
        let parsed = parse_message(raw);
        assert_eq!(parsed.from.as_deref(), Some("from@example.com"));
        assert_eq!(parsed.to, vec!["to@example.com"]);
        assert_eq!(parsed.cc, vec!["cc@example.com"]);
        assert_eq!(parsed.bcc, vec!["bcc@example.com"]);
        let sent = String::from_utf8_lossy(&parsed.raw_without_bcc);
        assert!(!sent.to_ascii_lowercase().contains("bcc"), "sent: {}", sent);
        assert!(sent.contains("Isi pesan."), "sent: {}", sent);
        assert!(sent.contains("Subject: Halo"), "sent: {}", sent);
    }

    #[test]
    fn parse_message_unfolds_continuation_lines() {
        let raw = b"To: satu@example.com,\r\n dua@example.com\r\nSubject: x\r\n\r\nbody";
        let parsed = parse_message(raw);
        assert_eq!(parsed.to, vec!["satu@example.com", "dua@example.com"]);
    }

    #[test]
    fn envelope_from_headers_with_t() {
        let cfg = SmtpConfig {
            host: "127.0.0.1".to_string(),
            port: 2525,
            security: SmtpSecurity::None,
            username: None,
            password: None,
            from: "cfg@example.com".to_string(),
        };
        let args = SendmailArgs {
            config: None,
            read_headers: true,
            from: None,
            recipients: Vec::new(),
        };
        let parsed = parse_message(b"From: a@example.com\nTo: b@example.com\nBcc: c@example.com\n\nisi");
        let (from, recipients) = build_envelope(&args, &parsed, &cfg).unwrap();
        assert_eq!(from.to_string(), "a@example.com");
        assert_eq!(recipients.len(), 2);

        // Tanpa -t dan tanpa penerima argumen: gagal jujur.
        let args = SendmailArgs::default();
        assert!(build_envelope(&args, &parsed, &cfg).is_err());
    }

    #[test]
    fn config_missing_fails_honestly() {
        std::env::remove_var("WEBFORGE_SMTP_CONFIG");
        let err = load_config(None).unwrap_err();
        assert!(err.contains("tidak ditemukan"), "err: {}", err);
        let err = load_config(Some("/tidak/ada/smtp.json")).unwrap_err();
        assert!(err.contains("tidak dapat dibaca"), "err: {}", err);
    }

    #[test]
    fn config_validation() {
        let cfg = SmtpConfig {
            host: "".to_string(),
            port: 587,
            security: SmtpSecurity::Starttls,
            username: None,
            password: None,
            from: "a@b.id".to_string(),
        };
        assert!(cfg.validate().is_err());
        let cfg = SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            security: SmtpSecurity::Tls,
            username: None,
            password: None,
            from: "bukan-email".to_string(),
        };
        assert!(cfg.validate().is_err());
    }
}
