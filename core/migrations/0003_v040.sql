-- v0.4.0: supervisi kesehatan berkelanjutan, peringatan docroot,
-- bukti probe terakhir, dan backend PHP internal untuk mode komposit.
ALTER TABLE server_instances ADD COLUMN warning_message TEXT;
ALTER TABLE server_instances ADD COLUMN last_health_at TEXT;
ALTER TABLE server_instances ADD COLUMN last_health_status INTEGER;
ALTER TABLE server_instances ADD COLUMN last_health_error TEXT;
ALTER TABLE server_instances ADD COLUMN last_health_target TEXT;
ALTER TABLE server_instances ADD COLUMN health_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE server_instances ADD COLUMN auto_restart_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE server_instances ADD COLUMN backend_port INTEGER;
