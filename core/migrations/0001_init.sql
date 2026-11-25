CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL,
    document_root TEXT NOT NULL DEFAULT '.',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project_frameworks (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    framework TEXT NOT NULL,
    language TEXT NOT NULL,
    version TEXT,
    recommended_runtime TEXT NOT NULL,
    recommended_document_root TEXT NOT NULL,
    readiness TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '[]',
    detected_files TEXT NOT NULL DEFAULT '[]',
    detected_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_project_frameworks_project ON project_frameworks(project_id);

CREATE TABLE IF NOT EXISTS server_instances (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    engine TEXT NOT NULL,
    port INTEGER NOT NULL,
    bind_lan INTEGER NOT NULL DEFAULT 0,
    document_root TEXT NOT NULL DEFAULT '.',
    status TEXT NOT NULL DEFAULT 'stopped',
    pid INTEGER,
    started_at TEXT,
    error_message TEXT,
    in_process INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_server_instances_project ON server_instances(project_id);
CREATE INDEX IF NOT EXISTS idx_server_instances_port ON server_instances(port);

CREATE TABLE IF NOT EXISTS server_configurations (
    id TEXT PRIMARY KEY,
    server_id TEXT NOT NULL REFERENCES server_instances(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_server_configurations_server ON server_configurations(server_id);

CREATE TABLE IF NOT EXISTS server_processes (
    id TEXT PRIMARY KEY,
    server_id TEXT NOT NULL REFERENCES server_instances(id) ON DELETE CASCADE,
    pid INTEGER,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    exit_code INTEGER
);
CREATE INDEX IF NOT EXISTS idx_server_processes_server ON server_processes(server_id);

CREATE TABLE IF NOT EXISTS port_allocations (
    id TEXT PRIMARY KEY,
    port INTEGER NOT NULL UNIQUE,
    server_id TEXT NOT NULL REFERENCES server_instances(id) ON DELETE CASCADE,
    allocated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_port_allocations_server ON port_allocations(server_id);

CREATE TABLE IF NOT EXISTS runtime_versions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    available INTEGER NOT NULL DEFAULT 0,
    version TEXT,
    reason TEXT,
    source TEXT NOT NULL DEFAULT 'none',
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS installed_extensions (
    id TEXT PRIMARY KEY,
    runtime_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT,
    available INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS python_environments (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    python_version TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS database_instances (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS database_connections (
    id TEXT PRIMARY KEY,
    database_id TEXT NOT NULL REFERENCES database_instances(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    connection_string TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS command_history (
    id TEXT PRIMARY KEY,
    command TEXT NOT NULL,
    exit_code INTEGER,
    stdout TEXT NOT NULL DEFAULT '',
    stderr TEXT NOT NULL DEFAULT '',
    duration_ms INTEGER NOT NULL DEFAULT 0,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_command_history_created ON command_history(created_at);

CREATE TABLE IF NOT EXISTS application_logs (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'info',
    message TEXT NOT NULL,
    server_id TEXT REFERENCES server_instances(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_application_logs_created ON application_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_application_logs_category ON application_logs(category);
CREATE INDEX IF NOT EXISTS idx_application_logs_server ON application_logs(server_id);

CREATE TABLE IF NOT EXISTS monitoring_metrics (
    id TEXT PRIMARY KEY,
    cpu_percent REAL,
    mem_used_bytes INTEGER,
    storage_used_bytes INTEGER,
    recorded_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_monitoring_metrics_recorded ON monitoring_metrics(recorded_at);

CREATE TABLE IF NOT EXISTS certificates (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backup_history (
    id TEXT PRIMARY KEY,
    database_id TEXT NOT NULL REFERENCES database_instances(id) ON DELETE CASCADE,
    backup_path TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_backup_history_db ON backup_history(database_id);

CREATE TABLE IF NOT EXISTS user_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT OR IGNORE INTO runtime_versions (id, name, available, version, reason, source, updated_at) VALUES
('static', 'WebForge Static Server', 1, '0.1.0', NULL, 'bundled', '1970-01-01T00:00:00Z'),
('php', 'PHP', 0, NULL, 'Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z'),
('nginx', 'Nginx', 0, NULL, 'Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z'),
('apache', 'Apache', 0, NULL, 'Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z'),
('python', 'Python', 0, NULL, 'Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z'),
('node', 'Node.js', 0, NULL, 'Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z'),
('mariadb', 'MariaDB', 0, NULL, 'Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z'),
('postgresql', 'PostgreSQL', 0, NULL, 'Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md', 'none', '1970-01-01T00:00:00Z');

INSERT OR IGNORE INTO user_settings (key, value, updated_at) VALUES
('theme', '"system"', '1970-01-01T00:00:00Z'),
('lan_default', 'false', '1970-01-01T00:00:00Z'),
('default_port', '8080', '1970-01-01T00:00:00Z'),
('editor_font_size', '14', '1970-01-01T00:00:00Z');
