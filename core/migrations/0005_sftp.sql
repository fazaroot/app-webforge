-- v0.5.0: pengaturan server SFTP (singleton, id selalu 1).
-- Password hanya disimpan sebagai hash Argon2id, tidak pernah plaintext.
CREATE TABLE IF NOT EXISTS sftp_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    username TEXT,
    password_hash TEXT,
    port INTEGER,
    bind_lan INTEGER,
    created_at TEXT,
    updated_at TEXT
);
