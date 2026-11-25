# WebForge Mobile — Kontrak API Phase 1 (final, semua subagent wajib ikut)

Root proyek: ~/workspace/WebForgeMobile
Struktur:
- core/        Rust crate `webforge-core` (lib cdylib+rlib + bin `webforge-core`)
- frontend/    React+TS+Vite (npm), build ke frontend/dist
- android/     Capacitor Android (dihasilkan `npx cap add android` lalu dikustomisasi Kotlin)
- package.json root: Capacitor CLI deps (mencontoh TeleCore)
- capacitor.config.ts root: appId com.webforge.mobile, webDir frontend/dist
- docs/, README.md, .github/workflows/android.yml, .gitignore root

## Core runtime
- Env: WEBFORGE_DATA_DIR (default ./webforge-data), WEBFORGE_WORKSPACE_DIR (default <data>/workspace),
  WEBFORGE_PORT (default 8765), WEBFORGE_API_TOKEN (opsional; bila kosong: baca/buat <data>/api.token mode 0600, token hex 32 byte)
- Bind management API: 127.0.0.1:WEBFORGE_PORT saja.
- JNI (cfg target_os=android, crate jni): Java_com_webforge_mobile_CoreBridge_nativeStart(env, dataDir, workspaceDir, port) -> jint (0 ok),
  nativeStop() -> jint, nativeGetToken() -> jstring. Library name: libwebforge_core.so (crate-type cdylib, lib name webforge_core).
- Bin lokal: `webforge-core` membaca env di atas, jalan foreground.

## Auth
Semua /api/* kecuali GET /api/health butuh header `X-API-Key: <token>` ATAU `Authorization: Bearer <token>`.
Tanpa token valid -> 401 {"error":"unauthorized"}.
CORS: izinkan origin capacitor://localhost, https://localhost, http://localhost:* (untuk dev). WebSocket terminal: token via query ?token=.

## Format respons
Sukses: JSON objek/array langsung (tanpa envelope). Error: {"error":"<pesan>"} + status HTTP tepat.
Semua timestamp: ISO8601 UTC string (chrono). ID: UUID v4 string.

## Endpoint (wajib semua)
- GET  /api/health -> {"status":"ok","version":"0.1.0","uptime_seconds":N}
- GET  /api/system/info -> {"version","os","arch","data_dir","workspace_dir","management_port","started_at","runtimes":[...]}
- GET  /api/system/diagnostics -> {"generated_at","checks":[{"name","status":"ok|warn|error","detail"}],"runtimes":[...],"ports":[...],"storage":{...}}
- GET  /api/runtimes -> [{"id":"static","name":"WebForge Static Server","available":true,"version":"0.1.0","reason":null},
  {"id":"php","name":"PHP","available":false,"version":null,"reason":"Belum dibundel di Phase 1 — lihat docs/RUNTIME_PLAN.md"},
  ... nginx, apache, python, node, mariadb, postgresql serupa false jujur]
  Deteksi nyata: cek binary di PATH/nativeLibraryDir bila ada -> available true + versi dari `--version` (timeout 3s). Di host dev: git/python3/node mungkin ada; tetap laporkan apa adanya dengan field "source":"path|bundled|none".
- GET  /api/projects -> [Project]
- POST /api/projects {name, template?: "static"|"native-php"|"flask"|...} -> buat folder workspace/<slug> + file template minimal nyata (static: index.html; native-php: index.php; flask: app.py+requirements.txt; dst — file nyata, bukan placeholder kosong) lalu scan. 201 Project.
- POST /api/projects/register {name, path} -> daftarkan path absolut di dalam workspace atau app-specific dir yang sudah ada; path di luar root terdaftar -> 400. Scan otomatis.
- GET  /api/projects/:id ; DELETE /api/projects/:id (hapus metadata saja, TIDAK hapus file; body {delete_files:true} untuk hapus folder workspace-nya)
- POST /api/projects/:id/scan -> hasil deteksi terbaru (ProjectFramework)
- Project: {"id","name","slug","path","document_root","created_at","updated_at","framework":{...}|null}
- Framework: {"framework":"Laravel|CodeIgniter 4|...|Static HTML|Unknown","language":"PHP|Python|Node.js|Static","version":null|"...",
  "recommended_runtime":"static|php|python|node","recommended_document_root":"public"|"."|...,"readiness":"ready|needs_runtime|needs_dependencies|unknown",
  "notes":["..."],"detected_files":["artisan",...]}
- GET  /api/servers -> [ServerInstance]
- POST /api/servers {project_id, engine:"static", port, bind_lan:bool, document_root?: rel path} -> 201. Engine selain static & runtime tidak tersedia -> 422 {"error":"Runtime X belum tersedia di Phase 1"}.
- GET  /api/servers/:id ; DELETE /api/servers/:id (harus stopped)
- POST /api/servers/:id/start|stop|restart -> ServerInstance. Start: cek konflik port (bind test + tabel port_allocations), spawn proses nyata,
  health check TCP connect + HTTP GET / (status <500) maks 5s; gagal -> status "error" + error_message, proses dibunuh. Status "running" HANYA bila health lolos.
- GET  /api/servers/:id/logs?limit=200 -> [{"id","level","message","created_at"}]
- ServerInstance: {"id","project_id","project_name","engine","port","bind_lan","document_root","status":"stopped|starting|running|stopping|error",
  "pid":null|N,"started_at":null|"...","uptime_seconds":N,"url_local":"http://127.0.0.1:8080","url_lan":null|"http://192.168.x.x:8080","error_message":null|"..."}
  Static engine: spawn proses anak `webforge-core serve-static --root <dir> --port <p> --host <127.0.0.1|0.0.0.0>` (subcommand bin yang sama; di Android JNI mode: task Tokio in-process dengan ServeDir + pid = pid proses sendiri dicatat jujur sebagai in-process? — di Android tetap spawn tidak bisa dari .so; gunakan in-process task dan pid null + field "in_process":true. Di host: spawn child nyata.)
- GET  /api/files/list?project_id=&path= (rel, default "") -> [{"name","path","is_dir","size","modified_at"}]
- GET  /api/files/read?project_id=&path= -> {"path","content","size"} (maks 1MB, binary -> 415)
- POST /api/files/write {project_id,path,content} -> {"ok":true,"size":N}
- POST /api/files/mkdir {project_id,path} ; POST /api/files/rename {project_id,from,to} ; POST /api/files/delete {project_id,path}
- POST /api/files/copy {project_id,from,to} ; POST /api/files/move {project_id,from,to}
- GET  /api/files/search?project_id=&q= -> [{"path","name","is_dir"}] (nama saja, rekursif, maks 200)
- GET  /api/files/dir-size?project_id=&path= -> {"path","bytes":N,"files":N}
  SEMUA path: canonicalize + wajib di dalam project.path; traversal -> 403.
- GET  /api/databases -> [{"id","name","path","size_bytes","created_at"}] (SQLite file di <data>/databases/<name>.db)
- POST /api/databases {name} ; DELETE /api/databases/:id
- GET  /api/databases/:id/tables -> [{"name","columns":[{"name","type","pk":bool}]}]
- POST /api/databases/:id/query {sql} -> {"columns":[...],"rows":[[...]],"rows_affected":N} (SELECT maupun DML; error SQL -> 400 dengan pesan sqlite)
- POST /api/databases/:id/backup -> {"backup_path","created_at"} ; POST /api/databases/:id/restore {backup_path} (path harus di folder backups)
- GET  /api/monitoring/current -> {"cpu_percent":f64,"mem_total_bytes":N,"mem_used_bytes":N,"storage_total_bytes":N,"storage_used_bytes":N,
  "processes":[{"pid","name","server_id":null|"..","uptime_seconds":N}],"ports":[{"port":N,"state":"LISTEN"}]}
  CPU dari /proc/stat (delta 2 sampel 200ms), memori /proc/meminfo, storage statvfs workspace, port /proc/net/tcp+tcp6 state 0A.
  Di host non-Linux/proc tidak ada -> nilai null + field "unavailable_reason". TIDAK ada angka karangan.
- GET  /api/monitoring/history?limit=60 -> [{"recorded_at","cpu_percent","mem_used_bytes","storage_used_bytes"}] (sampler tiap 30s ke monitoring_metrics)
- POST /api/terminal/execute {command, project_id?} -> {"exit_code":N,"stdout":"...","stderr":"...","duration_ms":N} (sh -c, cwd project/workspace, timeout 30s, simpan command_history)
- GET  /api/terminal/history?limit=50 -> [{"id","command","exit_code","created_at"}]
- WS   /ws/terminal?token= -> kirim {"type":"run","command":"...","project_id":null} terima {"type":"output","data":"..."} lalu {"type":"exit","code":N}
- GET  /api/network -> {"lan_ip":null|"192.168.x.x","interfaces":[{"name","ip"}],"management_url":"http://127.0.0.1:8765"}
- GET  /api/logs?severity=&search=&limit=200 -> [{"id","category","severity","message","created_at"}]
- GET  /api/logs/export -> text/plain unduhan log
- GET  /api/tools/git/status?project_id= -> {"available":bool,"reason":null|"...","branch":null|"..","clean":bool,"entries":[" M file",...]} (git status --porcelain nyata bila git ada)
- GET  /api/settings -> {"theme":"system","lan_default":false,...} ; PUT /api/settings {key:value,...}
- GET  /api/security/status -> {"api_token_set":true,"management_bind":"127.0.0.1","lan_servers":N,"warnings":["..."]}

## Skema SQLite (17 tabel, migrasi sqlx, FK ON, index wajar)
projects, project_frameworks, server_instances, server_configurations, server_processes, port_allocations,
runtime_versions, installed_extensions, python_environments, database_instances, database_connections,
command_history, application_logs, monitoring_metrics, certificates, backup_history, user_settings.
Tabel yang belum dipakai fitur Phase 1 (certificates, python_environments, installed_extensions, database_connections)
tetap dibuat + diisi registry runtime jujur (runtime_versions berisi static available=1, php/nginx/apache/python/node/mariadb/postgresql available=0 + reason).

## Frontend
- API base: dari bridge Kotlin (window.WebForgeBridge?.getApiBase()/getToken()) bila ada; fallback localStorage webforge_api_base (default http://127.0.0.1:8765) + webforge_token (diisi manual di Settings untuk dev). TanStack Query untuk semua data; Zustand untuk UI state (theme, project aktif, server aktif).
- 12 section via drawer/bottom-nav: Dashboard, Projects, Servers, Terminal, Databases, File Manager, Code Editor, Monitoring, Logs, Tools, Security, Settings. Semua tombol terhubung endpoint di atas; runtime unavailable tampil badge "Belum tersedia" + reason, TANPA tombol start.
- Code editor: CodeMirror 6 (@codemirror/*) edit+save nyata via /api/files/read|write.
- Monitoring chart: Recharts dari /api/monitoring/history + current polling 5s.
- Terminal: WS /ws/terminal streaming + fallback POST execute; history dari API.
- Warna: primary #5576E8, bg #F3F5FA, surface #FFFFFF, text #20283A, success #2EAD79, warning #E9A23B, danger #DC5866; dark bg #101522 surface #1A2233 text #E9EDF6. UI Bahasa Indonesia. StatusBar overlaysWebView:false (pelajaran TeleCore).
- Test: vitest — minimal test api client (auth header), store, dan 1 komponen render (mis. RuntimeBadge unavailable tidak menampilkan tombol start).

## Android
- Capacitor 7, appId com.webforge.mobile, minSdk 26 targetSdk 35 (variabel di android/variables.gradle sesuaikan).
- Kotlin: MainActivity (BridgeActivity), CoreBridge object (System.loadLibrary("webforge_core"), external fun), WebForgeService foreground service
  (notifikasi persisten status core, aksi Stop), plugin Capacitor WebForgePlugin: startCore/stopCore/getStatus/getToken/getApiBase/pickDirectory(SAF ACTION_OPEN_DOCUMENT_TREE -> salin tree ke workspace via DocumentFile, kembalikan path workspace) / openBatterySettings.
- JS bridge: frontend membaca token/base dari plugin (Capacitor registerPlugin('WebForge')) dengan fallback window.WebForgeBridge.
- jniLibs: CI menaruh libwebforge_core.so hasil cargo-ndk ke android/app/src/main/jniLibs/arm64-v8a/ (placeholder .gitkeep + README; JANGAN commit .so).
- Permission: INTERNET, FOREGROUND_SERVICE, FOREGROUND_SERVICE_SPECIAL_USE (atau DATA_SYNC — pilih specialUse dengan subtype jujur), POST_NOTIFICATIONS, WAKE_LOCK opsional. Tanpa MANAGE_EXTERNAL_STORAGE.

## CI (.github/workflows/android.yml)
Jobs: core (cargo test + build), frontend (npm ci/test/build), android (needs frontend+core: JDK21, Node22, Rust + cargo-ndk + target aarch64-linux-android, NDK dari runner (ANDROID_NDK_HOME), cargo ndk build, salin .so, cap sync, gradle assembleDebug, validasi APK berisi AndroidManifest.xml+classes.dex+lib/arm64-v8a/libwebforge_core.so, artifact webforge-debug-apk). TANPA setup-android action (pakai SDK bawaan runner, pelajaran TeleCore).

## Aturan jujur (semua)
Tidak ada mock data di jalur produksi, tidak ada tombol mati, status running hanya dari health check, runtime belum ada = unavailable + alasan.
