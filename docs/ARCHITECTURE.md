# Arsitektur WebForge Mobile — Phase 1

Dokumen ini menjelaskan arsitektur WebForge Mobile sebagaimana ditetapkan di [API_CONTRACT.md](../API_CONTRACT.md): empat lapis (Kotlin Android, jembatan JNI, Rust core, React frontend), model proses, model storage, keamanan, dan batasan background Android. Semua klaim di sini dibatasi pada Phase 1; runtime yang belum dibundel tidak digambarkan seolah sudah ada.

## Diagram 4 lapis

```
┌─────────────────────────────────────────────────────────────────┐
│ LAPIS 4 — React Frontend (WebView Capacitor)                    │
│ 12 section: Dashboard, Projects, Servers, Terminal, Databases,  │
│ File Manager, Code Editor, Monitoring, Logs, Tools, Security,   │
│ Settings. TanStack Query (data), Zustand (state UI),            │
│ CodeMirror 6 (editor), Recharts (grafik monitoring).            │
│ Memanggil management API lewat HTTP/WebSocket ke 127.0.0.1:8765.│
└──────────────────────────────┬──────────────────────────────────┘
                               │ HTTP + WebSocket (token API)
┌──────────────────────────────┴──────────────────────────────────┐
│ LAPIS 3 — Rust Core (crate webforge-core, libwebforge_core.so)  │
│ Axum + Tokio + SQLx/SQLite. Management API, static server,      │
│ deteksi framework, file manager, SQLite manager, terminal,      │
│ monitoring /proc, logs, diagnostics, registry runtime.          │
│ Bind management: 127.0.0.1:8765 saja.                           │
└──────────────────────────────┬──────────────────────────────────┘
                               │ JNI: nativeStart / nativeStop /
                               │      nativeGetToken
┌──────────────────────────────┴──────────────────────────────────┐
│ LAPIS 2 — Jembatan JNI (Kotlin CoreBridge + plugin Capacitor)   │
│ System.loadLibrary("webforge_core"). Plugin WebForge memberi    │
│ frontend: startCore, stopCore, getStatus, getToken, getApiBase,│
│ pickDirectory (SAF), openBatterySettings.                       │
└──────────────────────────────┬──────────────────────────────────┘
                               │ Android API
┌──────────────────────────────┴──────────────────────────────────┐
│ LAPIS 1 — Kotlin Android (Capacitor 7, appId com.webforge.mobile)│
│ MainActivity (BridgeActivity), WebForgeService (foreground      │
│ service + notifikasi persisten), SAF import via DocumentFile,   │
│ permission minimal. minSdk 26, targetSdk 35.                    │
└─────────────────────────────────────────────────────────────────┘
```

Alasan pemisahan ini:

- **Rust core memegang semua logika nyata** (server, deteksi, monitoring, database metadata) agar perilaku di host dev dan di Android identik dan bisa diuji dengan `cargo test` tanpa emulator.
- **Kotlin hanya mengurus hal yang memang milik Android**: lifecycle service, notifikasi, pemilih folder SAF, dan memuat library native.
- **React hanya tampilan.** Tidak ada data mock di jalur produksi; setiap tombol terhubung ke endpoint core. Runtime yang tidak tersedia ditampilkan dengan badge "Belum tersedia" dan alasannya, tanpa tombol Start.

## Kontrak JNI

Library native bernama `libwebforge_core.so` (crate-type `cdylib`, nama lib `webforge_core`), dibangun CI dengan `cargo-ndk` untuk `aarch64-linux-android` dan ditempatkan di `jniLibs/arm64-v8a/`. Fungsi yang diekspor (khusus `target_os = android`, crate `jni`):

| Fungsi | Peran |
|---|---|
| `Java_com_webforge_mobile_CoreBridge_nativeStart(env, dataDir, workspaceDir, port)` | Memulai core (runtime Tokio + Axum) dengan direktori data dan workspace dari Kotlin. Mengembalikan `0` bila berhasil. |
| `Java_com_webforge_mobile_CoreBridge_nativeStop()` | Menghentikan core secara bersih. |
| `Java_com_webforge_mobile_CoreBridge_nativeGetToken()` | Mengembalikan token API yang aktif agar frontend bisa mengautentikasi panggilan. |

Konfigurasi core dibaca dari environment: `WEBFORGE_DATA_DIR`, `WEBFORGE_WORKSPACE_DIR` (default `<data>/workspace`), `WEBFORGE_PORT` (default `8765`), dan `WEBFORGE_API_TOKEN` (opsional). Di host dev, binary `webforge-core` membaca env yang sama dan berjalan di foreground — jalur kode yang sama, tanpa JNI.

## Model proses

Ada dua jenis proses server di Phase 1: **management API** dan **static server per proyek**.

### Management API

- Satu instance core melayani management API di `127.0.0.1:8765` (port dari `WEBFORGE_PORT`). Bind ini **selalu localhost saja** — tidak ada opsi LAN untuk management API.
- Semua endpoint `/api/*` kecuali `GET /api/health` membutuhkan token (lihat bagian Keamanan).
- Metadata disimpan di SQLite (17 tabel: proyek, framework, instance server, alokasi port, registry runtime, riwayat perintah, log, metrik monitoring, pengaturan, dan tabel registry untuk fitur fase berikutnya seperti sertifikat). Tabel untuk fitur yang belum ada tetap dibuat dan diisi registry yang jujur — misalnya `runtime_versions` mencatat `static` tersedia dan PHP/Nginx/Apache/Python/Node/MariaDB/PostgreSQL tidak tersedia beserta alasannya.

### Static server

Perilaku start sama di semua platform: periksa konflik port (bind test + tabel `port_allocations`), mulai penyajian, lalu **health check** (koneksi TCP + HTTP GET `/` dengan status < 500, maksimal 5 detik). Status `running` hanya diberikan bila health check lolos; bila gagal, status menjadi `error` dengan `error_message` dan penyajian dihentikan.

Cara penyajiannya berbeda antara host dan Android, dan perbedaan ini dicatat apa adanya:

| Aspek | Host dev (Linux) | Android (di dalam aplikasi) |
|---|---|---|
| Mekanisme | Proses anak nyata: `webforge-core serve-static --root <dir> --port <p> --host <host>` | Task Tokio in-process di dalam core (ServeDir), karena binary terpisah tidak bisa di-spawn dari `.so` |
| PID | PID proses anak | `pid: null` dengan field `in_process: true` — tidak dikarang seolah ada proses terpisah |
| Bind | `127.0.0.1` default; `0.0.0.0` bila `bind_lan` aktif | Sama |
| Stop | Proses anak dimatikan | Task dibatalkan dan port dilepas |

Konsekuensi model in-process di Android: static server berbagi proses dengan core. Bila core berhenti (service dimatikan pengguna atau proses dibunuh OS), semua static server ikut berhenti. Ini jujur dan sederhana untuk Phase 1; runtime fase berikutnya yang berupa binary terpisah akan memakai model proses anak dari `nativeLibraryDir` (lihat [RUNTIME_PLAN.md](RUNTIME_PLAN.md)).

### Deteksi runtime

`GET /api/runtimes` tidak mengarang ketersediaan. Core memeriksa binary yang benar-benar ada di PATH/`nativeLibraryDir`; bila ditemukan, versi diambil dari `--version` dengan timeout 3 detik dan field `source` diisi `path` atau `bundled`. Bila tidak ada, `available: false` dengan alasan tertulis. Di host dev, git/python3/node mungkin terdeteksi dari PATH — itu dilaporkan apa adanya dan tidak berarti runtime tersebut dibundel di APK.

## Model storage: workspace vs SAF

Server web membutuhkan **path filesystem nyata**: document root, file statis, dan (nanti) interpreter semuanya bekerja dengan path POSIX biasa. Android modern tidak memberikan itu untuk folder sembarang — akses penyimpanan bersama diatur Storage Access Framework (SAF) yang berbasis URI `content://` dan izin per-folder, bukan path bebas. Karena itu WebForge Mobile memakai model workspace:

```
<data aplikasi>/
├── api.token            # token API, mode 0600
├── webforge.db          # metadata SQLite (proyek, server, log, dll.)
├── databases/           # file SQLite milik pengguna (<nama>.db)
├── backups/             # backup database
└── workspace/           # WEBFORGE_WORKSPACE_DIR
    ├── proyek-a/        # folder proyek (document root di dalamnya)
    └── proyek-b/
```

Aturan mainnya:

- **Proyek dibuat di workspace.** `POST /api/projects` membuat folder `workspace/<slug>` beserta file template nyata (mis. `index.html` untuk static, `index.php` untuk native-php, `app.py` + `requirements.txt` untuk flask), lalu memindainya.
- **SAF adalah pintu impor, bukan mount.** `pickDirectory` membuka `ACTION_OPEN_DOCUMENT_TREE`; setelah pengguna memilih folder, Kotlin menyalin tree tersebut ke workspace lewat `DocumentFile` dan mengembalikan path workspace-nya. Folder asal tidak diubah, dan perubahan di dalam aplikasi tidak menulis balik ke folder asal. Ini keterbatasan yang disengaja dan disampaikan apa adanya di UI.
- **Registrasi path dibatasi.** `POST /api/projects/register` hanya menerima path absolut yang sudah berada di dalam workspace atau direktori app-specific; path di luar root terdaftar ditolak (`400`).
- **Semua operasi file terkurung di proyek.** Setiap path di-canonicalize dan wajib berada di dalam `project.path`; upaya traversal (`../`) ditolak dengan `403`. Baca file dibatasi 1 MB dan file biner ditolak (`415`).
- **Tanpa `MANAGE_EXTERNAL_STORAGE`.** Aplikasi tidak meminta akses ke seluruh penyimpanan. Permission yang dipakai hanya `INTERNET`, `FOREGROUND_SERVICE` (+ tipe special use), `POST_NOTIFICATIONS`, dan `WAKE_LOCK` opsional.

Model ini menukar kenyamanan "edit folder di mana saja" dengan keamanan dan kepastian path: server selalu punya document root nyata, dan aplikasi tidak pernah menyentuh file di luar wilayahnya tanpa tindakan impor yang eksplisit dari pengguna.

## Keamanan

- **Token API.** Bila `WEBFORGE_API_TOKEN` kosong, core membaca atau membuat `<data>/api.token` dengan mode `0600` berisi token hex 32 byte. Token tidak pernah ditampilkan penuh di UI dan tidak ditulis ke log.
- **Autentikasi.** Semua `/api/*` kecuali `GET /api/health` membutuhkan header `X-API-Key: <token>` atau `Authorization: Bearer <token>`; tanpa token valid, respons `401 {"error":"unauthorized"}`. WebSocket terminal memakai token lewat query `?token=` karena browser tidak bisa memasang header khusus pada handshake WebSocket.
- **Bind localhost secara default.** Management API hanya di `127.0.0.1`. Static server juga `127.0.0.1` kecuali pengguna mengaktifkan `bind_lan` per server — pilihan opt-in yang membuka `0.0.0.0` dan menampilkan URL LAN. Endpoint `GET /api/security/status` melaporkan jumlah server LAN aktif dan peringatan yang relevan.
- **CORS terbatas.** Origin yang diizinkan hanya `capacitor://localhost`, `https://localhost`, dan `http://localhost:*` untuk pengembangan.
- **Port istimewa tidak dikejar.** Tanpa root, bind port < 1024 memang tidak bisa; WebForge Mobile tidak mencoba jalan pintas apa pun dan mengarahkan pengguna ke port ≥ 1024.
- **Terminal adalah permukaan sensitif.** Perintah berjalan sebagai UID aplikasi dengan `sh -c`, cwd di folder proyek/workspace, timeout 30 detik, dan riwayatnya tersimpan. Karena management API hanya bisa dijangkau dari perangkat itu sendiri dan butuh token, permukaan ini terbatas pada aplikasi dan pengguna perangkat.

## Background service dan batasan Android

Core berjalan di bawah `WebForgeService`, sebuah foreground service dengan notifikasi persisten yang menampilkan status core dan aksi Stop. Foreground service adalah cara sah Android untuk pekerjaan latar yang terlihat pengguna — tetapi ia **bukan jaminan uptime**:

- **Optimasi baterai.** OEM (termasuk HyperOS di POCO M6) dapat membatasi atau menghentikan aplikasi latar. Plugin menyediakan `openBatterySettings` agar pengguna bisa mengecualikan WebForge Mobile dari optimasi baterai, tetapi keputusan akhir tetap di tangan sistem dan pengguna.
- **Phantom-process killer.** Sejak Android 12, sistem membatasi proses "phantom" (proses anak yang tidak terdaftar sebagai komponen aplikasi) dan dapat membunuhnya bila melebihi batas jumlah atau pemakaian CPU. Model in-process Phase 1 mengurangi paparan ini dibanding model banyak proses anak, tetapi tidak menghilangkannya.
- **Proses bisa dimatikan OS kapan saja.** Tekanan memori, pembaruan aplikasi, atau force-stop akan menghentikan core dan semua servernya. Saat core mulai lagi, status server dibaca dari metadata dan tidak ada server yang diklaim `running` tanpa health check baru.
- **Layar mati ≠ server mati, tetapi juga bukan jaminan.** Selama foreground service hidup dan baterai tidak membatasi, static server tetap melayani. WebForge Mobile menyampaikan ini apa adanya: cocok untuk pengembangan dan pengujian di jaringan lokal, bukan pengganti server yang selalu menyala.

## W^X dan eksekusi native code

Sejak Android 10, kebijakan W^X (Write XOR Execute) yang ditegakkan SELinux melarang aplikasi mengeksekusi file dari direktori data yang bisa ditulis (`app_data_file`, mis. `getFilesDir()`). Satu-satunya lokasi yang boleh dieksekusi adalah `nativeLibraryDir` — direktori read-only tempat Android mengekstrak `.so` dari `jniLibs/<abi>/` di dalam APK saat instalasi.

Phase 1 sudah hidup di dalam aturan ini: satu-satunya kode native adalah `libwebforge_core.so` yang dimuat lewat `System.loadLibrary`, bukan dieksekusi sebagai binary terpisah. Aturan yang sama menentukan bentuk fase berikutnya: runtime seperti PHP atau Node hanya bisa dibundel sebagai file `lib*.so` di dalam APK dan dieksekusi dari `nativeLibraryDir`, dengan `extractNativeLibs`/`useLegacyPackaging` agar file benar-benar terekstrak ke disk. Mengunduh binary saat runtime lalu menjalankannya dari folder data **tidak** bisa dan **tidak** akan dicoba — pembaruan runtime berarti pembaruan APK. Pembahasan lengkapnya ada di [RUNTIME_PLAN.md](RUNTIME_PLAN.md).
