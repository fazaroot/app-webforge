# jniLibs/arm64-v8a

Folder ini menerima `libwebforge_core.so` hasil cross-compile Rust core
(`cargo ndk -t arm64-v8a ... build --release --lib`) dari CI
(`.github/workflows/android.yml`).

File `.so` TIDAK di-commit ke repository. Build lokal tanpa file ini akan
menghasilkan APK yang gagal memuat library native; aplikasi melaporkan
kegagalan itu apa adanya (lihat `CoreBridge.loadError`).
