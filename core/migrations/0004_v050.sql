-- v0.5.0: pilihan versi PHP per proyek/server, backend PHP-FPM untuk
-- komposit Apache/Nginx/Caddy, dan rekomendasi versi PHP dari
-- constraint composer.json.
ALTER TABLE projects ADD COLUMN php_version TEXT;
ALTER TABLE server_instances ADD COLUMN php_version TEXT;
ALTER TABLE server_instances ADD COLUMN backend_kind TEXT;
ALTER TABLE project_frameworks ADD COLUMN php_version_constraint TEXT;
ALTER TABLE project_frameworks ADD COLUMN recommended_php_version TEXT;
