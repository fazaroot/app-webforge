//! Enkripsi kredensial database eksternal/lokal.
//!
//! Password database tidak pernah disimpan plaintext dan tidak pernah
//! dikembalikan ke frontend. Kunci AES-256-GCM digenerate sekali dan
//! disimpan di file `db-secret.key` (0600) di data dir privat aplikasi.
//! Format ciphertext: hex(nonce 12 byte || ciphertext+tag).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use std::path::{Path, PathBuf};

fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("db-secret.key")
}

fn load_or_create_key(data_dir: &Path) -> Result<[u8; 32], String> {
    let path = key_path(data_dir);
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("Gagal menulis kunci enkripsi: {}", e))?;
        f.write_all(&key)
            .map_err(|e| format!("Gagal menulis kunci enkripsi: {}", e))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, key).map_err(|e| format!("Gagal menulis kunci enkripsi: {}", e))?;
    }
    Ok(key)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("Ciphertext tidak valid".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "Ciphertext tidak valid".to_string()))
        .collect()
}

pub fn encrypt_secret(data_dir: &Path, plaintext: &str) -> Result<String, String> {
    let key = load_or_create_key(data_dir)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| "Enkripsi kredensial gagal".to_string())?;
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(to_hex(&combined))
}

pub fn decrypt_secret(data_dir: &Path, encoded: &str) -> Result<String, String> {
    let key = load_or_create_key(data_dir)?;
    let combined = from_hex(encoded)?;
    if combined.len() <= 12 {
        return Err("Ciphertext tidak valid".to_string());
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| "Dekripsi kredensial gagal".to_string())?;
    String::from_utf8(plaintext).map_err(|_| "Kredensial terdekripsi tidak valid".to_string())
}

/// Password acak untuk layanan database lokal (hex 24 byte).
pub fn generate_password() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    to_hex(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let dir = std::env::temp_dir().join(format!("wf-secrets-test-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let enc = encrypt_secret(&dir, "kata-sandi-rahasia").unwrap();
        assert!(!enc.contains("kata-sandi"));
        let dec = decrypt_secret(&dir, &enc).unwrap();
        assert_eq!(dec, "kata-sandi-rahasia");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
