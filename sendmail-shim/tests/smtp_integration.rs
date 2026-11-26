//! Integrasi: server SMTP plain palsu di localhost menerima pesan
//! dari shim (tanpa TLS, tanpa auth).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use webforge_sendmail_shim::{run, SendmailArgs};

/// Jalankan server SMTP palsu satu koneksi. Mengembalikan port dan
/// channel berisi transkrip perintah + isi DATA.
fn fake_smtp_server() -> (u16, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut transcript = String::new();
        let _ = writer.write_all(b"220 fake.local ESMTP siap\r\n");
        let mut in_data = false;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).unwrap_or(0);
            if n == 0 {
                break;
            }
            transcript.push_str(&line);
            let upper = line.to_ascii_uppercase();
            if in_data {
                if line.trim_end_matches(['\r', '\n']) == "." {
                    in_data = false;
                    let _ = writer.write_all(b"250 2.0.0 pesan diterima\r\n");
                }
                continue;
            }
            if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                let _ = writer.write_all(b"250 fake.local\r\n");
            } else if upper.starts_with("MAIL FROM") {
                let _ = writer.write_all(b"250 2.1.0 ok\r\n");
            } else if upper.starts_with("RCPT TO") {
                let _ = writer.write_all(b"250 2.1.5 ok\r\n");
            } else if upper.starts_with("DATA") {
                in_data = true;
                let _ = writer.write_all(b"354 akhiri dengan titik tunggal\r\n");
            } else if upper.starts_with("QUIT") {
                let _ = writer.write_all(b"221 2.0.0 sampai jumpa\r\n");
                break;
            } else if upper.starts_with("RSET") || upper.starts_with("NOOP") {
                let _ = writer.write_all(b"250 ok\r\n");
            } else {
                let _ = writer.write_all(b"502 perintah tidak dikenal\r\n");
            }
        }
        let _ = tx.send(transcript);
    });
    (port, rx)
}

fn write_config(port: u16) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wf-sendmail-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("smtp.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"host":"127.0.0.1","port":{},"security":"none","username":null,"password":null,"from":"pengirim@example.com"}}"#,
            port
        ),
    )
    .unwrap();
    path
}

#[test]
fn sends_plain_smtp_with_t_headers() {
    let (port, rx) = fake_smtp_server();
    let config = write_config(port);
    let args: Vec<String> = vec![
        "--config".to_string(),
        config.to_string_lossy().to_string(),
        "-t".to_string(),
        "-i".to_string(),
    ];
    let message = "From: pengirim@example.com\r\nTo: tujuan@example.com\r\nBcc: rahasia@example.com\r\nSubject: Uji SMTP\r\n\r\nHalo dari WebForge.\r\n";
    let result = run(&args, message.as_bytes());
    assert!(result.is_ok(), "run gagal: {:?}", result.err());

    let transcript = rx.recv().unwrap();
    assert!(transcript.contains("MAIL FROM"), "transkrip: {}", transcript);
    assert!(transcript.contains("tujuan@example.com"), "transkrip: {}", transcript);
    // Bcc masuk envelope tetapi tidak muncul di isi DATA.
    assert!(transcript.contains("rahasia@example.com"), "transkrip: {}", transcript);
    let data_section = transcript.split("DATA").nth(1).unwrap_or("");
    assert!(!data_section.to_ascii_lowercase().contains("bcc:"), "data: {}", data_section);
    assert!(data_section.contains("Halo dari WebForge."), "data: {}", data_section);
}

#[test]
fn sends_to_explicit_recipient_without_t() {
    let (port, rx) = fake_smtp_server();
    let config = write_config(port);
    let args: Vec<String> = vec![
        "--config".to_string(),
        config.to_string_lossy().to_string(),
        "-f".to_string(),
        "pengirim@example.com".to_string(),
        "langsung@example.com".to_string(),
    ];
    let message = "Subject: Tanpa header penerima\r\n\r\nIsi.\r\n";
    let result = run(&args, message.as_bytes());
    assert!(result.is_ok(), "run gagal: {:?}", result.err());
    let transcript = rx.recv().unwrap();
    assert!(transcript.contains("langsung@example.com"), "transkrip: {}", transcript);
}

#[test]
fn missing_config_fails() {
    let args: Vec<String> = vec!["-t".to_string()];
    let result = run(&args, b"To: a@b.id\n\nisi");
    assert!(result.is_err());
}

#[test]
fn args_struct_default_is_empty() {
    let args = SendmailArgs::default();
    assert!(args.recipients.is_empty());
    assert!(!args.read_headers);
    // Pastikan Read terpakai di helper (kompilasi bersih tanpa warning aneh).
    let _ = std::io::empty().read(&mut [0u8; 0]);
}
