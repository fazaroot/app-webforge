# Rencana Bundel Runtime — WebForge Mobile

Dokumen ini adalah rencana fase berikutnya untuk membundel runtime bahasa dan server ke dalam APK WebForge Mobile, berdasarkan riset singkat atas proyek yang benar-benar ada. Isinya rencana dan penilaian kelayakan — **bukan** klaim bahwa runtime apa pun sudah tersedia. Di Phase 1 semuanya berstatus "Belum tersedia — belum dibundel" (lihat [COMPATIBILITY.md](COMPATIBILITY.md)).

Aturan main yang tidak bisa ditawar, berlaku untuk semua runtime di dokumen ini:

1. **Tanpa root.** Semua runtime berjalan sebagai UID aplikasi biasa, port ≥ 1024.
2. **Patuh W^X.** Sejak Android 10, file di direktori data yang bisa ditulis tidak boleh dieksekusi. Binary runtime harus dibundel di APK sebagai `lib*.so` di `jniLibs/arm64-v8a/` dan dieksekusi dari `nativeLibraryDir`. Tidak ada unduhan binary saat runtime, tidak ada penurunan `targetSdk` untuk menghindari aturan.
3. **Bukti sebelum klaim.** Runtime baru dinyatakan tersedia hanya setelah binary-nya benar-benar ada di APK, terdeteksi registry (`source: "bundled"`), lolos health check di perangkat uji (POCO M6), dan versinya terbaca dari `--version`.
4. **Lisensi dihormati.** Setiap runtime membawa lisensinya sendiri; APK harus menyertakan pemberitahuan lisensi yang diwajibkan, dan kewajiban copyleft (GPL) dipenuhi sebelum distribusi, bukan sesudah.

## Ringkasan riset

Temuan dari riset (sumber lengkap di bagian akhir):

- **PHP bisa di-cross-compile untuk Android ARM64.** Ada bukti nyata: proyek swoole-cli menyediakan "PHP Runtime Layer" `libphp.a` untuk Android `arm64-v8a` yang dibangun dengan NDK; ada pula build PHP yang dibundel ke aplikasi Android sebagai `php.so` di `jniLibs` dan dijalankan dengan `php -S`, lengkap dengan penyesuaian `LD_LIBRARY_PATH` dan `TMPDIR` (Android tidak punya `/tmp`). php-src sendiri tidak menyediakan target Android resmi, jadi patch dan penyesuaian build system tetap pekerjaan sendiri.
- **NDK mendukung build system asing secara resmi.** Dokumentasi NDK menjelaskan cara memakai Clang NDK dengan Autoconf dan Makefile non-Android (target `aarch64-linux-android<api>`), yang merupakan jalur untuk nginx, PHP, dan pustaka C lain. Ini jalur yang didukung Google, bukan akal-akalan.
- **Node.js punya jalur mobile resmi-ish: nodejs-mobile.** Proyek ini mendistribusikan `libnode.so` untuk `arm64-v8a` (API ≥ 24, NDK ≥ r24) yang dijalankan in-process lewat `node::Start` di thread latar. Dokumentasinya menunjukkan pola menyalin proyek Node ke `FilesDir` lalu menjalankannya — cocok dengan model workspace WebForge.
- **Python punya distribusi CPython Android yang terawat: Chaquopy.** Chaquopy adalah plugin Gradle + runtime yang menanam CPython (3.8–3.13) ke APK untuk `arm64-v8a` dan `x86_64`, dan disebut sebagai satu-satunya sumber binary CPython Android siap pakai yang aktif terawat; sejak 2025 berlisensi Apache-2.0 dan diterbitkan ke Maven Central. CPython upstream juga makin mendukung Android, tetapi Chaquopy tetap jalur tercepat.
- **MariaDB dan PostgreSQL terbukti jalan di userspace Android — lewat Termux.** Repositori Termux menyediakan paket `mariadb` dan `postgresql` aarch64 yang berjalan sebagai server di bawah UID aplikasi biasa (port tinggi, data dir di prefix aplikasi, layanan runit), lengkap dengan shim `libandroid-shmem`/`libandroid-execinfo` untuk PostgreSQL. Ada juga laporan PostgreSQL 18 berjalan di dalam aplikasi Android dengan binary yang disamarkan sebagai `lib*.so` di `jniLibs` dan shared memory dipaksa ke `mmap`. Bukti kelayakan ada; biaya bundelnya yang besar.
- **W^X menentukan bentuk bundel.** Analisis SELinux menunjukkan `nativeLibraryDir` adalah satu-satunya lokasi yang boleh dieksekusi (`exec_type`), file di sana harus bernama `lib*.so`, harus terekstrak ke disk (`extractNativeLibs`/`useLegacyPackaging`), dan binary harus memakai interpreter linker Android (`/system/bin/linker64`, Bionic) — binary glibc biasa tidak akan jalan. Desain APKLF Termux menegaskan hal yang sama dari sisi proyek yang paling terdampak aturan ini.

## Kendala bersama semua runtime

- **Packaging `.so`.** Setiap executable runtime dibungkus sebagai `lib<namanya>.so` di `jniLibs/arm64-v8a/`, dengan `android:extractNativeLibs="true"` (atau `useLegacyPackaging`) agar file benar-benar ada di disk untuk di-`execve`. Pustaka pendamping diletakkan di direktori yang sama dan dirujuk lewat `LD_LIBRARY_PATH` yang menunjuk `nativeLibraryDir`.
- **Bionic, bukan glibc.** Semua binary dan pustaka harus di-build melawan Bionic libc Android dengan toolchain NDK. Binary Linux ARM64 biasa (glibc) tidak kompatibel.
- **Path prefix tidak ada.** Runtime yang mengasumsikan `/usr`, `/etc`, `/tmp`, atau `/var/run` harus diarahkan ke direktori data aplikasi lewat flag konfigurasi dan environment (`TMPDIR`, `HOME`, prefix compile-time bila bisa, atau config file yang dihasilkan core). Core yang menghasilkan config dari template dengan path workspace adalah pola yang akan dipakai.
- **Satu APK, satu versi.** Memperbarui runtime berarti merilis APK baru. Tidak ada manajer paket di dalam aplikasi.
- **Ukuran APK.** Setiap runtime menambah puluhan MB (perkiraan kasar; angka pasti wajib diukur dari build nyata). Bundel semua runtime sekaligus tidak realistis untuk satu APK; urutan prioritas di bawah menentukan apa yang masuk lebih dulu, dan pemisahan per-runtime (mis. unduhan modul saat instalasi lewat Play feature delivery) hanya dipertimbangkan bila distribusinya memang lewat Play Store.
- **Siklus hidup proses.** Runtime fase berikutnya berjalan sebagai proses anak dari `nativeLibraryDir`, diawasi core: spawn, health check yang sama seperti static server, tangkap stdout/stderr ke log per-server, bunuh saat stop. Phantom-process killer Android 12+ membatasi proses anak — jumlah server yang berjalan bersamaan harus dibatasi dan statusnya jujur bila proses dibunuh sistem.

## Rencana per runtime

### PHP — prioritas pertama

- **Jalur:** cross-compile php-src dengan NDK (target `aarch64-linux-android26` agar selaras `minSdk 26`), menghasilkan binary CLI yang dibungkus `libphp.so`. Bukti jalur ini ada dari swoole-cli (runtime layer Android `arm64-v8a`) dan build PHP bundel Android yang menjalankan `php -S` dari `nativeLibraryDir`.
- **Tahap 1: `php -S` (built-in server).** Cukup untuk native PHP dan pengembangan Laravel/CodeIgniter skala lokal. Core men-spawn `libphp.so -S 127.0.0.1:<port> -t <document_root>`, health check HTTP seperti static server.
- **Tahap 2: PHP-FPM.** Setelah CLI terbukti, build SAPI FPM dan pasangkan dengan Nginx/Caddy. FPM butuh socket/TCP dan manajemen pool — lebih banyak permukaan kegagalan, jadi tidak didahulukan.
- **Ekstensi realistis tahap awal:** inti yang dibutuhkan framework umum dan tidak menarik dependensi native berat: `json`, `mbstring`, `tokenizer`, `xml`/`dom`, `pdo` + `pdo_sqlite`, `sqlite3`, `openssl`, `curl`, `zip`, `fileinfo`, `ctype`, `filter`, `session`, `hash`. Ekstensi yang berat/bermasalah di Bionic — `intl` (ICU besar), `gd` (rantai libpng/freetype), `imagick`, `redis`, `swoole` — ditunda dan hanya masuk bila build-nya terbukti, bukan dijanjikan.
- **Penyesuaian wajib:** `TMPDIR` ke cache aplikasi, `php.ini` dihasilkan core (session save path, upload tmp dir, error log ke workspace), `LD_LIBRARY_PATH` untuk pustaka pendamping (mis. `libsqlite3` bila tidak statis). Composer tidak dibundel di tahap awal; dependensi proyek diasumsikan sudah ada di folder proyek (`vendor/`) — `readiness: needs_dependencies` tetap dipakai bila `vendor/` tidak ada.
- **Risiko:** patch php-src untuk Bionic harus dirawat sendiri setiap naik versi PHP; ekstensi PECL di luar daftar inti tidak dijanjikan.

### Python — prioritas kedua

- **Jalur utama: Chaquopy.** Plugin Gradle menanam CPython `arm64-v8a` beserta stdlib dan ekstensi C inti (`_ssl`, `_sqlite3`, `_ctypes`, dll.) ke APK. Ini jalur tercepat dan terawat; lisensi Apache-2.0 sejak 2025 memudahkan bundel.
- **Jalur alternatif: CPython upstream.** CPython makin mendukung Android secara resmi, tetapi merakit distribusi sendiri (stdlib, ekstensi C, OpenSSL, SQLite) adalah pekerjaan besar; hanya dipilih bila Chaquopy mentok.
- **Model jalan:** interpreter in-process di thread terpisah (pola Chaquopy), menjalankan server pengembangan framework: Flask (`flask run` / `app.run`), FastAPI (`uvicorn`), Django (`runserver`). Karena in-process, satu interpreter per proses aplikasi; restart server berarti restart interpreter — core harus mengelola ini dengan jujur (server Python kedua yang bersamaan mungkin tidak didukung di tahap awal).
- **Dependensi proyek:** wheel murni-Python bisa disertakan dari folder proyek; wheel dengan ekstensi C (mis. `pydantic` core, `psycopg`, `numpy`) hanya jalan bila ada wheel Android yang kompatibel — umumnya **tidak ada**. `readiness: needs_dependencies` dipakai untuk proyek yang dependensinya tidak terpenuhi, bukan dipaksakan.
- **Risiko:** ukuran APK (interpreter + stdlib), konflik versi Python proyek vs versi bundel, dan keterbatasan wheel native adalah batasan nyata yang akan terlihat di penggunaan.

### Node.js — prioritas ketiga

- **Jalur: nodejs-mobile.** `libnode.so` `arm64-v8a` dijalankan in-process lewat `node::Start` di thread latar, dengan entrypoint proyek di workspace (pola resmi nodejs-mobile: salin proyek ke direktori file aplikasi, jalankan dari sana). Server pengembangan Express/NestJS/Next.js dev server adalah target tahap awal.
- **Batasan yang diketahui:** Node in-process berarti satu instance Node per proses aplikasi; menghentikan server Node dengan bersih dan menjalankannya lagi dalam proses yang sama adalah titik rawan yang harus diuji di perangkat sebelum diklaim. Modul native npm (`.node`) harus di-build untuk Android — sebagian besar tidak tersedia; proyek yang bergantung padanya berstatus `needs_dependencies`.
- **npm tidak dibundel sebagai manajer paket penuh** di tahap awal; `node_modules` diasumsikan sudah ada di folder proyek. Menjalankan `npm install` di perangkat membutuhkan npm + registry + build tools native, yang tidak realistis dijanjikan.
- **Risiko:** ukuran `libnode.so` besar; versi Node mengikuti rilis nodejs-mobile, bukan rilis Node terbaru.

### Nginx — setelah PHP-FPM

- **Jalur:** cross-compile nginx dengan NDK memakai panduan build system asing NDK (Autoconf/Makefile). nginx pada dasarnya portabel dan Termux membuktikan nginx jalan di Android userspace.
- **Modul minimal:** HTTP core, `proxy`, `fastcgi` (untuk PHP-FPM), `rewrite`, `gzip`, `ssl` (OpenSSL NDK) bila HTTPS lokal dibutuhkan. Modul dinamis tidak dipakai; semua statis agar tidak bergantung pada `dlopen` dari lokasi non-`nativeLibraryDir`.
- **Penyesuaian:** prefix diarahkan ke direktori data (`-p`, `-c`, `-g` untuk pid/log/temp path), worker process 1, tanpa `user` directive (tidak ada root/setuid). Config dihasilkan core dari template per proyek.
- **Nilai:** pasangan Nginx + PHP-FPM adalah stack yang paling mirip produksi untuk Laravel/WordPress. Karena itu urutannya setelah PHP terbukti, bukan sebelumnya.

### Caddy — kandidat realistis, nilai lebih rendah

- **Jalur:** Go mendukung `GOOS=android GOARCH=arm64` dengan cgo memakai NDK; Caddy adalah binary Go tunggal sehingga cross-compile-nya paling sederhana di antara semua kandidat server. Binary dibungkus `libcaddy.so` seperti runtime lain.
- **Nilai:** file server + reverse proxy + HTTPS otomatis adalah fitur khas Caddy, tetapi ACME membutuhkan domain publik yang bisa divalidasi dari internet — dari HP di jaringan lokal itu umumnya tidak terpenuhi. Nilai praktis tahap awalnya adalah reverse proxy di depan PHP-FPM/Node dan penyajian statis, yang sebagian sudah ditutup static server Rust.
- **Keputusan:** dikerjakan setelah PHP/Python/Node, atau lebih awal hanya bila kebutuhan reverse-proxy terbukti mendesak.

### Apache httpd — penilaian jujur: sulit, prioritas terendah

- httpd bergantung pada APR/APR-util dan mengasumsikan model proses Unix klasik (prefork, setuid ke `www-data`, modul dinamis `mod_so`). Termux memang menyediakan `apache2`, jadi "bisa jalan di Android" terbukti — tetapi memaketkannya sebagai bundel rapi di APK lain soal: banyak modul, config yang mengasumsikan prefix sistem, dan `mod_php` yang menanam PHP ke proses httpd adalah kombinasi yang paling rapuh untuk dirawat.
- Nilai khas Apache untuk pengguna WebForge adalah `.htaccess` (WordPress dan hosting bersama). Nilai itu nyata, tetapi biaya porting + perawatan jauh di atas Nginx/Caddy + PHP-FPM untuk hasil yang setara di kebanyakan proyek.
- **Keputusan:** tidak dijadwalkan. Ditinjau ulang hanya bila PHP-FPM + Nginx terbukti dan kebutuhan `.htaccess` menjadi penghambat utama pengguna.

### MariaDB — penilaian jujur: berat, mungkin tidak dibundel

- **Kelayakan teknis:** terbukti jalan di Termux sebagai server userspace (data dir di prefix aplikasi, socket di prefix, port tinggi, dependensi seperti OpenSSL/PCRE2/zstd). Membundelnya ke APK berarti membangun seluruh rantai dependensi itu dengan NDK dan membungkus banyak binary (`mariadbd`, `mariadb-install-db`, client) sebagai `lib*.so`, plus inisialisasi data dir saat pertama jalan.
- **Biaya:** ukuran APK besar, pemakaian RAM dan I/O yang signifikan untuk HP, inisialisasi dan pemulihan crash yang harus dirawat, dan **lisensi GPLv2** — distribusi binary MariaDB memicu kewajiban copyleft (menyediakan source yang bersesuaian dan mempertahankan lisensi), yang harus dipenuhi secara eksplisit sebelum APK dibagikan.
- **Keputusan:** tidak dibundel di fase dekat. SQLite menutup kebutuhan database lokal Phase 1–2. MariaDB hanya dipertimbangkan bila kebutuhan MySQL-compat terbukti kuat, dan saat itu kewajiban GPL dipenuhi sebagai bagian dari rilis, bukan catatan kaki.

### PostgreSQL — penilaian jujur: mungkin, tetapi mahal

- **Kelayakan teknis:** Termux menyediakan PostgreSQL aarch64 dengan shim `libandroid-shmem` dan `libandroid-execinfo` (Android tidak menyediakan SysV shared memory yang diasumsikan PostgreSQL). Laporan independen menunjukkan PostgreSQL 18 berjalan di dalam aplikasi Android: binary dibungkus `lib*.so` di `jniLibs`, `argv[0]` diarahkan lewat symlink `bin/`, dan shared memory dipaksa ke `mmap` (`shared_memory_type=mmap`, `dynamic_shared_memory_type=mmap`) plus preload shim.
- **Biaya:** banyak binary pendamping (`postgres`, `initdb`, `pg_ctl`, `psql`), inisialisasi cluster, pengaturan shared memory yang rapuh, dan ukuran APK. Lisensinya permisif (PostgreSQL License), jadi hambatannya teknis, bukan hukum.
- **Keputusan:** kandidat fase jauh, setelah runtime bahasa terbukti. Alternatif yang lebih jujur untuk kebutuhan "database server" di HP tetap SQLite; PostgreSQL dibundel hanya bila ada kebutuhan nyata yang tidak bisa dipenuhi SQLite.

### Yang tidak direncanakan

- **proot/distro Linux penuh.** Menjalankan rootfs distro lewat proot (gaya Termux `proot-distro`) memberi fleksibilitas besar tetapi bertabrakan dengan model W^X targetSdk modern (Termux sendiri terpaksa bertahan di `targetSdk 28` untuk itu dan sedang mengerjakan desain APKLF sebagai pengganti). WebForge Mobile tidak akan menurunkan targetSdk atau meniru jalan itu; bundel per-runtime dari `nativeLibraryDir` adalah jalur yang dipilih.
- **Unduhan runtime saat aplikasi berjalan.** Dilarang oleh W^X dan tidak akan diakali.
- **Port 80/443, systemd, Docker.** Tidak tersedia tanpa root di Android dan tidak dijanjikan dalam bentuk apa pun.

## Urutan prioritas fase berikutnya

| Urutan | Pekerjaan | Alasan urutan |
|---|---|---|
| 1 | PHP CLI + `php -S`, ekstensi inti | Nilai tertinggi: membuka native PHP, Laravel, CodeIgniter. Jalur build terbukti ada. |
| 2 | Python via Chaquopy (Flask/FastAPI/Django dev server) | Distribusi CPython Android terawat; membuka stack Python tanpa merakit interpreter sendiri. |
| 3 | Node.js via nodejs-mobile (Express/NestJS dev server) | `libnode.so` siap pakai; batasan satu instance dan modul native harus diuji di perangkat. |
| 4 | PHP-FPM + Nginx modul minimal | Stack mirip produksi untuk PHP; masuk akal hanya setelah PHP terbukti. |
| 5 | Caddy | Cross-compile Go paling mudah; nilai utamanya reverse proxy, bukan ACME dari jaringan lokal. |
| 6 | PostgreSQL (evaluasi ulang) | Mungkin secara teknis, mahal; hanya bila kebutuhan nyata muncul. |
| — | Apache httpd, MariaDB | Tidak dijadwalkan: Apache karena biaya porting vs nilai, MariaDB karena berat + kewajiban GPL. Ditinjau ulang berdasarkan kebutuhan terbukti. |

Setiap tahap selesai hanya bila: binary ada di APK hasil CI, registry melaporkan `available: true` dengan `source: "bundled"` dan versi nyata, server lolos health check di POCO M6, dan [COMPATIBILITY.md](COMPATIBILITY.md) diperbarui pada rilis yang sama.

## Risiko

| Risiko | Dampak | Mitigasi |
|---|---|---|
| W^X / SELinux | Binary tidak bisa dieksekusi bila salah packaging; kebijakan Android bisa mengetat | Bundel hanya sebagai `lib*.so` di `nativeLibraryDir`, `extractNativeLibs` aktif, uji exec di perangkat nyata setiap rilis; tidak ada jalur unduhan runtime |
| Ukuran APK | Setiap runtime menambah puluhan MB; APK membengkak dan instalasi melambat | Bundel bertahap sesuai prioritas, ukur ukuran dari build nyata sebelum memutuskan, pertimbangkan hanya `arm64-v8a` |
| Lisensi | GPL (MariaDB) mewajibkan source; lisensi lain mewajibkan atribusi | Sertakan berkas lisensi pihak ketiga di APK/repo, penuhi kewajiban GPL sebelum distribusi, catat versi + sumber tiap runtime |
| Perawatan patch | php-src/CPython/nginx tidak menargetkan Android secara resmi; patch bisa basi tiap naik versi | Pin versi runtime, simpan patch di repo, naikkan versi hanya dengan uji perangkat |
| Phantom-process killer & baterai OEM | Server anak dibunuh sistem tanpa peringatan | Batasi server bersamaan, health check berkala, status jujur saat proses mati, panduan pengecualian baterai |
| Modul/dependensi native proyek | Wheel Python / modul Node / ekstensi PECL tidak tersedia untuk Android | `readiness: needs_dependencies` yang jujur; tidak menjanjikan `pip install`/`npm install`/`composer` di perangkat |
| Keamanan LAN | Server LAN bisa dijangkau siapa pun di jaringan | Bind localhost default untuk semua runtime; LAN tetap opt-in per server dengan peringatan |

## Sumber riset

URL di bawah disalin persis dari hasil pencarian yang dipakai untuk menyusun rencana ini:

- PHP bundel Android (`php.so` di `jniLibs`, `php -S`, `LD_LIBRARY_PATH`/`TMPDIR`): https://re-ws.pl/2024/12/php-build-for-use-bundled-in-android-applications/
- swoole-cli — PHP Runtime Layer Android `arm64-v8a` (`libphp.a`, NDK): https://git.junyouji.com/swoole/swoole-cli
- Build PHP 8.5 per ABI dengan NDK (embed SAPI): https://git.jumphp.net/jumphp/jumphp2installer/src/branch/main/tools/android/build.sh
- Panduan resmi NDK untuk build system asing (Autoconf/Makefile, target `aarch64-linux-android`): https://developer.android.com/ndk/guides/other_build_systems?authuser=00
- nodejs-mobile — panduan Android (`libnode.so`, `arm64-v8a`, API ≥ 24): https://nodejs-mobile.github.io/docs/guide/guide-android/getting-started/
- nodejs-mobile — proyek kompleks (salin proyek ke `FilesDir`, modul npm): https://nodejs-mobile.github.io/docs/guide/guide-android/building-complex/
- Chaquopy sebagai sumber CPython Android terawat (Apache-2.0, Maven Central, isi distribusi): https://mob-dev.hexdocs.pm/0.7.17/MobDev.PythonAndroidSupport.html
- Chaquopy runtime (CPython 3.8–3.13, matriks ABI): https://deepwiki.com/coidarashka/MandreLib/2.1-chaquopy:-python-runtime
- MariaDB & PostgreSQL di Termux (paket, data dir, shim `libandroid-shmem`): https://github.com/RedSigilDelta/termux-forge/blob/HEAD/08-power-tools/10-databases.md
- Stack Termux di HP lama (php/nginx/apache2/mariadb/postgresql tersedia, phantom-process killer): https://shounok.fyi/nextcloud-on-old-android-phone/
- PostgreSQL 18 di dalam aplikasi Android (`lib*.so`, shared memory `mmap`): https://dev.to/pocketcode/how-i-got-postgresql-18-running-inside-an-android-app-5chj
- W^X/SELinux dan eksekusi dari `nativeLibraryDir`: https://dev.to/ai2th/pockr-part-2-executing-binaries-on-android-3b4k
- Desain APKLF Termux (konteks `targetSdk 28` dan batasan Android 10+): https://deepwiki.com/termux/termux.github.io/8.3-apk-library-file-(apklf)-design
