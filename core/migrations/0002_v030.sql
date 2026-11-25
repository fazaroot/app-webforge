-- v0.3.0: konfigurasi server per engine + layanan database non-SQLite.
ALTER TABLE server_instances ADD COLUMN config_text TEXT;
ALTER TABLE server_instances ADD COLUMN proxy_port INTEGER;
ALTER TABLE server_instances ADD COLUMN command TEXT;

ALTER TABLE database_instances ADD COLUMN kind TEXT NOT NULL DEFAULT 'sqlite';
ALTER TABLE database_instances ADD COLUMN host TEXT;
ALTER TABLE database_instances ADD COLUMN port INTEGER;
ALTER TABLE database_instances ADD COLUMN username TEXT;
ALTER TABLE database_instances ADD COLUMN password_encrypted TEXT;
ALTER TABLE database_instances ADD COLUMN database_name TEXT;
ALTER TABLE database_instances ADD COLUMN status TEXT NOT NULL DEFAULT 'stopped';
ALTER TABLE database_instances ADD COLUMN data_dir TEXT;
ALTER TABLE database_instances ADD COLUMN error_message TEXT;

INSERT OR IGNORE INTO runtime_versions (id, name, available, version, reason, source, updated_at) VALUES
('caddy', 'Caddy', 0, NULL, 'Belum dibundel — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z');
