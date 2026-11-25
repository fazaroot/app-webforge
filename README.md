# WebForge Mobile

Lingkungan server web dan pengembangan yang berjalan **langsung di HP Android**, tanpa root dan tanpa PC. WebForge Mobile membundel core Rust di dalam aplikasi Android (lewat JNI), menyajikan antarmuka React di WebView Capacitor, dan mengelola proyek web yang tersimpan di workspace milik aplikasi.

- **Application ID:** `com.webforge.mobile`
- **Versi Phase 1:** `0.1.0`
- **Platform:** Android `minSdk 26` (Android 8.0), `targetSdk 35`, fokus ARM64 (`arm64-v8a`)
- **Perangkat uji utama:** Xiaomi POCO M6 (ARM64)
- **Build:** GitHub Actions (APK debug), tanpa Android Studio lokal

> **Status jujur Phase 1:** yang benar-benar berjalan adalah **static server Rust**, deteksi framework, file manager, SQLite manager, terminal terbatas, monitoring `/proc`, logs, dan diagnostics. Runtime PHP, Nginx, Apache, Python, Node.js, MariaDB, dan PostgreSQL **belum dibundel** — aplikasi menampilkannya sebagai "Belum tersedia" beserta alasannya, bukan sebagai tombol mati. Rencana bundelnya ada di [docs/RUNTIME_PLAN.md](docs/RUNTIME_PLAN.md), dan matriks lengkapnya di [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md).

## Fitur Phase 1 yang nyata

Semua fitur di bawah terhubung ke endpoint core yang nyata. Status server `running` hanya diberikan setelah health check lolos (koneksi TCP + HTTP GET `/` dengan status < 500, maksimal 5 detik); tidak ada status karangan.

| Fitur | Keterangan |
|---|---|
| Static server Rust | Menyajikan file statis dari document root proyek. Di Android berjalan sebagai task in-process di core; di host dev sebagai proses anak `webforge-core serve-static`. Port bisa dipilih, bind LAN bersifat opt-in. |
| Deteksi framework | Pemindaian file proyek mengenali 17 pola framework/stack (Static HTML, Native PHP, Laravel, CodeIgniter 4, Symfony, WordPress, Flask, Django, FastAPI, Express, NestJS, Next.js, Nuxt, Vue, React, Angular, SvelteKit) dan melaporkan `readiness`: `ready`, `needs_runtime`, `needs_dependencies`, atau `unknown`. Deteksi bukan berarti bisa dijalankan — proyek PHP terdeteksi dengan benar tetapi tetap `needs_runtime` selama PHP belum dibundel. |
| File manager + code editor | List, baca (maks. 1 MB, file biner ditolak), tulis, buat folder, rename, copy, move, delete, pencarian nama rekursif, dan ukuran direktori. Semua path divalidasi agar tetap di dalam folder proyek; traversal ditolak. Editor memakai CodeMirror 6 dan menyimpan lewat API yang sama. |
| SQLite manager | Buat/hapus database SQLite di folder data aplikasi, lihat tabel dan kolom, jalankan query SQL (SELECT maupun DML), backup, dan restore dari folder backup. |
| Terminal | Eksekusi perintah `sh -c` di folder proyek/workspace dengan timeout 30 detik, streaming lewat WebSocket, dan riwayat perintah. Ini shell Android yang terbatas — bukan bash Linux lengkap dan tidak ada manajer paket. |
| Monitoring | CPU dari `/proc/stat`, memori dari `/proc/meminfo`, storage dari statvfs workspace, port LISTEN dari `/proc/net/tcp`. Bila `/proc` tidak tersedia, nilai dilaporkan `null` dengan alasan — tidak ada angka karangan. Riwayat disimpan tiap 30 detik untuk grafik. |
| Logs & diagnostics | Log aplikasi dan per-server dengan filter severity/pencarian dan ekspor teks. Diagnostics menjalankan pemeriksaan nyata (runtime, port, storage) dengan status `ok`/`warn`/`error`. |
| Status Git | `git status --porcelain` nyata **bila** binary git terdeteksi. Di Android Phase 1 git umumnya tidak ada, jadi fitur ini melaporkan `available: false` dengan alasan. |
| Registry runtime jujur | Endpoint `/api/runtimes` mendeteksi binary yang benar-benar ada (PATH/`nativeLibraryDir`) dan melaporkan versi dari `--version`. Runtime yang belum dibundel dilaporkan `available: false` dengan alasan tertulis. |

## Cara build

Build resmi dilakukan di GitHub Actions, karena pengembangan dilakukan tanpa PC/Android Studio:

1. Push ke branch `main` (atau jalankan workflow manual) memicu `.github/workflows/android.yml`.
2. Job `core`: `cargo test` + build Rust.
3. Job `frontend`: `npm ci`, test, dan build Vite ke `frontend/dist`.
4. Job `android`: JDK 21, Node 22, Rust + `cargo-ndk` untuk target `aarch64-linux-android`, hasil `libwebforge_core.so` disalin ke `jniLibs/arm64-v8a/`, Capacitor sync, lalu Gradle `assembleDebug`.
5. APK divalidasi berisi `AndroidManifest.xml`, `classes.dex`, dan `lib/arm64-v8a/libwebforge_core.so`, lalu diunggah sebagai artifact `webforge-debug-apk`.

File `.so` hasil build **tidak** di-commit ke repo; CI yang membuatnya. Untuk pengembangan core di host Linux biasa:

```bash
cd core
cargo test
cargo run --bin webforge-core   # management API di 127.0.0.1:8765
```

## Cara pakai di HP

1. Install APK hasil CI di HP, lalu buka aplikasi. Core Rust mulai otomatis lewat foreground service (ada notifikasi persisten berisi status core dan aksi Stop).
2. Buat proyek dari menu Projects, atau impor folder yang sudah ada lewat pemilih folder (SAF). Impor **menyalin** isi folder ke workspace aplikasi — folder asal di penyimpanan HP tidak diubah dan tidak diedit langsung.
3. Semua proyek hidup di workspace milik aplikasi (`<data>/workspace/<slug>`). Model ini disengaja: server butuh path filesystem nyata untuk document root, dan Android tidak memberi aplikasi akses bebas ke seluruh penyimpanan.
4. Buka proyek, periksa hasil deteksi framework, lalu buat server static dari menu Servers dan tekan Start. URL lokal (`http://127.0.0.1:<port>`) bisa dibuka di browser HP.
5. Gunakan port di atas 1024 (mis. 8080). Port istimewa seperti 80/443 tidak bisa di-bind tanpa root.
6. Bind LAN bersifat opt-in per server. Aktifkan hanya di jaringan tepercaya: siapa pun di jaringan yang sama bisa membuka situs yang disajikan. Management API sendiri selalu hanya di `127.0.0.1:8765` dan dilindungi token.
7. Agar server tidak mudah dimatikan sistem, izinkan aplikasi berjalan di latar belakang (nonaktifkan optimasi baterai untuk WebForge Mobile). Android tetap berhak menghentikan proses; lihat batasan di bawah.

## Batasan jujur Phase 1

- **Runtime bahasa belum dibundel.** PHP/PHP-FPM, Nginx, Apache, Caddy, Python (Flask/Django/FastAPI), Node.js/npm, MariaDB, dan PostgreSQL belum tersedia. Proyek dengan stack tersebut bisa dideteksi dan file-nya bisa diedit, tetapi belum bisa dijalankan.
- **Tanpa root, tanpa port istimewa.** Server hanya bisa memakai port ≥ 1024.
- **Android bukan OS server.** Foreground service membantu, tetapi optimasi baterai, pembatas proses latar belakang, dan phantom-process killer (Android 12+) tetap bisa menghentikan proses. WebForge Mobile tidak menjanjikan uptime seperti server Linux sungguhan.
- **Workspace adalah salinan.** Folder dari SAF disalin masuk; perubahan di aplikasi tidak menulis balik ke folder asal, dan sebaliknya.
- **Terminal terbatas.** Shell Android (`sh`) tanpa paket Linux; perintah seperti `apt`, `composer`, atau `npm` tidak tersedia di Phase 1.
- **Belum ada HTTPS/ACME.** Static server Phase 1 melayani HTTP saja.
- **Eksekusi binary dibatasi W^X.** Sejak Android 10, aplikasi tidak boleh mengeksekusi file dari direktori data yang bisa ditulis; kode native yang dibundel harus berada di `nativeLibraryDir` sebagai `.so` di dalam APK. Konsekuensinya: runtime baru hanya bisa datang lewat update APK, bukan unduhan saat aplikasi berjalan. Detailnya di [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) dan [docs/RUNTIME_PLAN.md](docs/RUNTIME_PLAN.md).

## Struktur repo

```
WebForgeMobile/
├── API_CONTRACT.md          # Kontrak API Phase 1 (sumber kebenaran bersama)
├── README.md
├── core/                    # Rust crate webforge-core (cdylib + rlib + bin)
│   └── src/                 # Axum API, static server, deteksi, monitoring, JNI
├── frontend/                # React + TypeScript + Vite (build ke frontend/dist)
├── android/                 # Capacitor Android + Kotlin (service, bridge JNI, SAF)
├── docs/
│   ├── ARCHITECTURE.md      # Arsitektur 4 lapis, model proses & storage, keamanan
│   ├── COMPATIBILITY.md     # Matriks fitur: berfungsi / parsial / belum tersedia
│   └── RUNTIME_PLAN.md      # Rencana bundel PHP, Python, Node, Nginx, dll.
├── capacitor.config.ts      # appId com.webforge.mobile, webDir frontend/dist
├── package.json             # Dependensi Capacitor CLI
└── .github/workflows/android.yml
```

## Dokumentasi

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — bagaimana Kotlin, JNI, Rust core, dan React bekerja sama; model proses; storage; keamanan; batasan background Android.
- [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) — status jujur tiap fitur dan runtime di Phase 1.
- [docs/RUNTIME_PLAN.md](docs/RUNTIME_PLAN.md) — riset dan rencana bundel runtime fase berikutnya, termasuk risiko W^X, ukuran APK, dan lisensi.
- [API_CONTRACT.md](API_CONTRACT.md) — kontrak endpoint, skema, dan aturan kejujuran yang mengikat semua bagian.
