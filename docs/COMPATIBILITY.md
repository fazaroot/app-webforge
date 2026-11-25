# Kompatibilitas WebForge Mobile — Phase 1

Matriks ini menyatakan status jujur setiap fitur dan runtime di Phase 1 (`0.1.0`, `com.webforge.mobile`, Android `minSdk 26` / `targetSdk 35`, ARM64). Status yang dipakai:

- **Berfungsi** — terimplementasi nyata dan terhubung ke core; perilaku sesuai keterangan.
- **Parsial** — sebagian berfungsi dengan batasan nyata yang dijelaskan; batasan tidak disembunyikan.
- **Belum tersedia — belum dibundel** — tidak ada di APK Phase 1. Aplikasi menampilkan status ini beserta alasannya dan tidak menyediakan tombol Start. Rencana bundelnya ada di [RUNTIME_PLAN.md](RUNTIME_PLAN.md).

Prinsip yang mengikat seluruh matriks: status server `running` hanya dari health check yang lolos, deteksi runtime hanya dari binary yang benar-benar ada, dan angka monitoring hanya dari `/proc` yang benar-benar terbaca. Tidak ada mock data di jalur produksi.

## Matriks fitur

| Fitur | Status Phase 1 | Alasan / keterangan |
|---|---|---|
| Static server (Rust) | **Berfungsi** | Menyajikan file dari document root proyek. Di Android berjalan sebagai task in-process (`in_process: true`, `pid: null`); di host dev sebagai proses anak `webforge-core serve-static`. Start memeriksa konflik port lalu health check (TCP + HTTP GET `/` status < 500, maks. 5 detik); gagal berarti status `error`, bukan `running`. Bind `127.0.0.1` default, LAN opt-in per server. HTTP saja, port ≥ 1024. |
| Deteksi 17 framework | **Parsial** | Deteksi berbasis file berfungsi untuk 17 pola: Static HTML, Native PHP, Laravel, CodeIgniter 4, Symfony, WordPress, Flask, Django, FastAPI, Express, NestJS, Next.js, Nuxt, Vue, React, Angular, SvelteKit (plus `Unknown` sebagai fallback). Hasilnya mencakup bahasa, runtime yang direkomendasikan, document root yang disarankan, file penanda yang ditemukan, dan `readiness`. Parsial karena deteksi ≠ bisa jalan: proyek PHP/Python/Node terdeteksi dengan benar tetapi `readiness: needs_runtime` selama runtime-nya belum dibundel. Deteksi adalah heuristik file, bukan analisis kode penuh; versi framework hanya dilaporkan bila terbaca dari file penanda. |
| File manager | **Berfungsi** | List, baca (maks. 1 MB; biner ditolak `415`), tulis, mkdir, rename, copy, move, delete, pencarian nama rekursif (maks. 200 hasil), dan ukuran direktori — semua di dalam folder proyek. Path di-canonicalize; traversal keluar proyek ditolak `403`. Code editor (CodeMirror 6) memakai endpoint baca/tulis yang sama. Wilayah kerja adalah workspace aplikasi, bukan seluruh penyimpanan HP. |
| SQLite manager | **Berfungsi** | Database SQLite dibuat di `<data>/databases/<name>.db`; daftar tabel/kolom, query SELECT maupun DML (error SQL dikembalikan apa adanya), backup, dan restore dari folder backup. Ini manajer untuk file SQLite milik aplikasi — bukan server database jaringan. |
| Terminal | **Parsial** | Eksekusi `sh -c` nyata di cwd proyek/workspace, timeout 30 detik, streaming WebSocket, riwayat tersimpan. Parsial karena shell Android sangat terbatas: bukan bash, tidak ada manajer paket, tidak ada `composer`/`npm`/`pip`, dan binary yang bisa dipanggil hanya yang ada di sistem/`nativeLibraryDir`. |
| Monitoring | **Parsial** | CPU dari delta `/proc/stat` (2 sampel, 200 ms), memori dari `/proc/meminfo`, storage dari statvfs workspace, port LISTEN dari `/proc/net/tcp`+`tcp6` (state `0A`), riwayat tiap 30 detik. Parsial karena bergantung pada keterbacaan `/proc` di perangkat: bila tidak tersedia, nilai `null` + `unavailable_reason`, bukan angka karangan. Sebagian OEM membatasi visibilitas proses aplikasi lain, jadi daftar proses bisa lebih pendek dari kenyataan sistem. |
| Logs | **Berfungsi** | Log aplikasi dan per-server tersimpan di SQLite, bisa difilter severity/pencarian, dan diekspor sebagai teks. Log static server berasal dari proses/task yang benar-benar berjalan. |
| Diagnostics | **Berfungsi** | `GET /api/system/diagnostics` menjalankan pemeriksaan nyata (runtime terdeteksi, port, storage) dengan status `ok`/`warn`/`error` per pemeriksaan. Diagnostics melaporkan kondisi apa adanya, termasuk runtime yang belum tersedia. |
| Git status | **Parsial** | `git status --porcelain` nyata **bila** binary git terdeteksi di PATH/`nativeLibraryDir`. Di APK Android Phase 1 git tidak dibundel, jadi umumnya `available: false` dengan alasan. Di host dev yang punya git, fitur ini berfungsi penuh. |
| Background service | **Parsial** | Foreground service dengan notifikasi persisten menjaga core tetap hidup selama sistem mengizinkan, dan ada jalan ke pengaturan baterai. Parsial karena Android tetap berhak menghentikan proses: optimasi baterai OEM, batas proses latar, dan phantom-process killer (Android 12+) bisa mematikan core beserta servernya. Tidak ada janji uptime. |
| SAF (impor folder) | **Parsial** | Pemilih folder `ACTION_OPEN_DOCUMENT_TREE` berfungsi dan menyalin tree terpilih ke workspace lewat `DocumentFile`, lalu proyek didaftarkan dari salinan itu. Parsial karena modelnya salin-masuk: folder asal tidak diedit langsung, perubahan tidak menulis balik, dan aplikasi tidak meminta `MANAGE_EXTERNAL_STORAGE`. Server membutuhkan path nyata, dan SAF tidak menyediakan path bebas — lihat [ARCHITECTURE.md](ARCHITECTURE.md). |

## Matriks runtime dan layanan

| Runtime / layanan | Status Phase 1 | Alasan / keterangan |
|---|---|---|
| PHP | **Belum tersedia — belum dibundel** | Tidak ada binary PHP di APK. Proyek PHP (termasuk template `native-php` yang membuat `index.php` nyata) terdeteksi sebagai `needs_runtime`. Rencana: cross-compile php-src dengan NDK, mulai dari `php -S` — lihat [RUNTIME_PLAN.md](RUNTIME_PLAN.md). |
| PHP-FPM | **Belum tersedia — belum dibundel** | Bergantung pada PHP yang belum dibundel. Direncanakan setelah `php -S` terbukti, dipasangkan dengan Nginx/Caddy. |
| Apache httpd | **Belum tersedia — belum dibundel** | Tidak dibundel. Penilaian jujur di rencana runtime: porting httpd + APR ke Android berat dan `mod_php` tidak realistis; prioritas rendah dibanding Nginx/Caddy + PHP-FPM. |
| Nginx | **Belum tersedia — belum dibundel** | Tidak ada binary Nginx di APK. Static server Rust menutup kebutuhan file statis Phase 1. Rencana cross-compile modul minimal ada di [RUNTIME_PLAN.md](RUNTIME_PLAN.md). |
| Caddy | **Belum tersedia — belum dibundel** | Tidak dibundel di Phase 1. Secara teknis realistis di fase berikutnya (Go `android/arm64`), tetapi belum dikerjakan dan tidak diklaim tersedia. |
| Python / Flask / Django / FastAPI | **Belum tersedia — belum dibundel** | Tidak ada interpreter Python di APK. Deteksi file berfungsi (mis. `app.py`, `requirements.txt`, `manage.py`) dan template flask membuat file nyata, tetapi aplikasi Python belum bisa dijalankan. Rencana: Chaquopy/CPython — lihat [RUNTIME_PLAN.md](RUNTIME_PLAN.md). |
| Node.js / npm | **Belum tersedia — belum dibundel** | Tidak ada `libnode` maupun npm di APK. Proyek Node terdeteksi dari `package.json` dan sejenisnya, berstatus `needs_runtime`/`needs_dependencies`. Rencana: nodejs-mobile — lihat [RUNTIME_PLAN.md](RUNTIME_PLAN.md). |
| MariaDB | **Belum tersedia — belum dibundel** | Tidak ada server MariaDB/MySQL. Kebutuhan database Phase 1 dilayani SQLite. Penilaian bundel MariaDB (berat, lisensi GPL) ada di [RUNTIME_PLAN.md](RUNTIME_PLAN.md). |
| PostgreSQL | **Belum tersedia — belum dibundel** | Tidak ada server PostgreSQL di APK. Terbukti bisa jalan di userspace Android oleh proyek lain, tetapi belum dibundel di sini dan tidak diklaim tersedia. |
| SSL / ACME (HTTPS) | **Belum tersedia — belum dibundel** | Static server Phase 1 melayani HTTP saja. Tabel `certificates` ada di skema sebagai registry, tetapi penerbitan/penyajian HTTPS belum diimplementasikan. ACME juga membutuhkan domain publik dan tantangan yang bisa dijangkau internet — tidak realistis dari HP di jaringan lokal tanpa tunnel. |

## Catatan perangkat dan platform

- **Arsitektur:** APK Phase 1 menargetkan `arm64-v8a`. Perangkat uji utama adalah Xiaomi POCO M6 (ARM64, HyperOS). Perilaku OEM lain — terutama soal pembunuhan proses latar — bisa berbeda dan belum terverifikasi di luar perangkat uji.
- **Versi Android:** `minSdk 26`, `targetSdk 35`. Kebijakan W^X Android 10+ berlaku penuh: kode native hanya dieksekusi dari `nativeLibraryDir`.
- **Port:** hanya port ≥ 1024. Management API tetap di `127.0.0.1:8765` dan tidak bisa diakses dari jaringan.
- **Deteksi di host dev bukan bukti bundel.** Bila core dijalankan di Linux dan menemukan `git`/`python3`/`node` di PATH, registry melaporkannya dengan `source: "path"`. Itu kondisi host, bukan isi APK.
- **Kesiapan proyek:** `readiness: ready` di Phase 1 pada praktiknya hanya untuk proyek static. Nilai `needs_runtime` dan `needs_dependencies` adalah sinyal jujur bahwa proyek terdeteksi tetapi prasyarat jalannya belum terpenuhi di perangkat.
