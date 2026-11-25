#!/usr/bin/env bash
# WebForge Mobile — cross-compile PostgreSQL 16 untuk Android arm64 (API 26).
#
# Output (di $OUT_DIR):
#   libpostgres.so, libinitdb.so, libpgctl.so, libpsql.so (bila terbuild)
#   postgres-share.zip   (isi: share/postgresql/... dan lib/postgresql/... bila ada)
#   postgres-version.txt
#
# Binary dikemas sebagai lib*.so agar sah dieksekusi dari nativeLibraryDir
# (W^X Android 10+). Di perangkat, core membuat symlink bin/postgres ->
# libpostgres.so di home instance dan menjalankannya dengan argv[0] path
# symlink itu; build ini mematikan HAVE_PROC_SELF_EXE agar PostgreSQL
# menyelesaikan share/ relatif terhadap argv[0], bukan /proc/self/exe.
# Lihat core/src/db_runtime.rs untuk kontrak runtime-nya.
#
# Keputusan build:
# - Versi dipin ke 16.3 + patch Termux dari commit yang sama yang
#   memaketkan 16.3, supaya patch Android Termux terbukti cocok dengan
#   source-nya. Naik patch-level (16.x lebih baru) wajib menguji ulang
#   patch tersebut.
# - Dependensi statis: OpenSSL + zlib dibangun dari source; readline,
#   libxml2, ICU, GSSAPI, LDAP, PAM, Perl/Python/Tcl, LZ4, ZSTD dimatikan.
#   Konsekuensi: psql tanpa line-editing, tipe/fungsi XML tidak tersedia,
#   collation hanya C/POSIX + libc (initdb memakai --locale=C).
# - Semaphore POSIX unnamed (USE_UNNAMED_POSIX_SEMAPHORES=1): System V
#   semaphore tidak tersedia di Android. Shared memory dipaksa mmap di
#   config runtime (db_runtime.rs), bukan SysV shmget / POSIX shm_open
#   yang tidak disediakan Bionic/kernel Android.
# - libandroid-shmem/libandroid-execinfo (shim Termux) dibangun statis
#   bila source-nya ada di checkout Termux; build tetap lanjut tanpa
#   keduanya karena jalur mmap + semaphore unnamed tidak membutuhkannya.
#   backtrace() PostgreSQL hanya aktif bila shim execinfo terbuild.
#
# Script ini TIDAK pernah dijalankan di sandbox lokal (tidak ada NDK);
# bukti pertama adalah run CI. Kegagalan apa pun dibuat keras (exit 1)
# dengan nama langkah/patch yang gagal — jangan longgarkan gate ELF di
# bawah hanya agar CI hijau.
set -euo pipefail

PG_VERSION="${PG_VERSION:-16.3}"
PG_SHA256="${PG_SHA256:-331963d5d3dc4caf4216a049fa40b66d6bcb8c730615859411b9518764e60585}"
OPENSSL_VERSION="${OPENSSL_VERSION:-3.3.2}"
ZLIB_VERSION="${ZLIB_VERSION:-1.3.1}"
ANDROID_API="${ANDROID_API:-26}"
# Commit termux-packages yang memaketkan PostgreSQL 16.3
# (packages/postgresql/build.sh: TERMUX_PKG_VERSION="16.3").
TERMUX_COMMIT="${TERMUX_COMMIT:-ff9d0d08b7274dd2923b90e6c6437aa5d0cc573d}"
# Prefix placeholder yang dibakar ke binary. Path nyata di perangkat
# berbeda per instance, jadi runtime tidak boleh bergantung pada prefix
# ini: share dicari relatif terhadap argv[0] symlink (lihat atas).
PG_PREFIX_PLACEHOLDER="${PG_PREFIX_PLACEHOLDER:-/webforge/pg16}"

WORK_DIR="${WORK_DIR:-$PWD/db-build/postgres}"
OUT_DIR="${OUT_DIR:-$PWD/postgres-out}"
PREFIX="$WORK_DIR/prefix"
SRC_DIR="$WORK_DIR/src"
STAGE_DIR="$WORK_DIR/stage"

log() { echo "[build-postgres] $*"; }
fail() { echo "::error::[build-postgres] $*" >&2; exit 1; }

mkdir -p "$WORK_DIR" "$OUT_DIR" "$PREFIX/lib/pkgconfig" "$PREFIX/include" "$SRC_DIR"

for tool in curl tar patch git perl make zip readelf file sha256sum; do
  command -v "$tool" >/dev/null || fail "tool host tidak ditemukan: $tool"
done
command -v flex >/dev/null || log "peringatan: flex tidak ada (tarball rilis PG sudah berisi file hasil generate)"
command -v bison >/dev/null || log "peringatan: bison tidak ada (tarball rilis PG sudah berisi file hasil generate)"

# --- Toolchain NDK (pola yang sama dengan job build-php di android.yml) ---
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  NDK_DIR=$(ls -d "${ANDROID_HOME:-}"/ndk/* 2>/dev/null | sort -V | tail -1 || true)
  [ -n "$NDK_DIR" ] || fail "Android NDK tidak ditemukan (ANDROID_NDK_HOME kosong dan \$ANDROID_HOME/ndk tidak ada)"
  ANDROID_NDK_HOME="$NDK_DIR"
fi
TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
[ -x "$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang" ] || fail "clang NDK tidak ditemukan di $TOOLCHAIN"
export PATH="$TOOLCHAIN/bin:$PATH"
export CC="$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang"
export CXX="$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang++"
export AR=llvm-ar
export RANLIB=llvm-ranlib
export STRIP=llvm-strip
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
"$CC" --version | head -1

# --- zlib statis (NDK sysroot tidak menyediakan zlib.pc) ---
if [ ! -f "$PREFIX/lib/libz.a" ]; then
  log "build zlib $ZLIB_VERSION (statis)"
  cd "$SRC_DIR"
  curl -fsSL -o zlib.tar.gz "https://github.com/madler/zlib/releases/download/v${ZLIB_VERSION}/zlib-${ZLIB_VERSION}.tar.gz"
  tar xf zlib.tar.gz
  cd "zlib-${ZLIB_VERSION}"
  CC="$CC" AR="$AR" RANLIB="$RANLIB" ./configure --prefix="$PREFIX" --static
  make -j"$(nproc)"
  make install
fi

# --- OpenSSL statis ---
if [ ! -f "$PREFIX/lib/libssl.a" ]; then
  log "build OpenSSL $OPENSSL_VERSION (statis)"
  cd "$SRC_DIR"
  curl -fsSL -o openssl.tar.gz "https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VERSION}/openssl-${OPENSSL_VERSION}.tar.gz"
  tar xf openssl.tar.gz
  cd "openssl-${OPENSSL_VERSION}"
  ./Configure android-arm64 "-D__ANDROID_API__=${ANDROID_API}" \
    no-shared no-tests no-docs \
    --prefix="$PREFIX" --openssldir="$PREFIX/ssl"
  make -j"$(nproc)" build_sw
  make install_sw
fi

# --- Source PostgreSQL ---
cd "$SRC_DIR"
if [ ! -d "postgresql-${PG_VERSION}" ]; then
  log "unduh PostgreSQL $PG_VERSION"
  curl -fsSL -o postgresql.tar.bz2 "https://ftp.postgresql.org/pub/source/v${PG_VERSION}/postgresql-${PG_VERSION}.tar.bz2"
  echo "${PG_SHA256}  postgresql.tar.bz2" | sha256sum -c - || fail "SHA256 PostgreSQL tidak cocok"
  tar xf postgresql.tar.bz2
fi
cd "postgresql-${PG_VERSION}"

# --- Patch Android Termux (sparse checkout, commit dipin) ---
TERMUX_DIR="$WORK_DIR/termux-packages"
if [ ! -d "$TERMUX_DIR/packages/postgresql" ]; then
  log "ambil patch Termux @ $TERMUX_COMMIT"
  mkdir -p "$TERMUX_DIR"
  cd "$TERMUX_DIR"
  git init -q
  git remote add origin https://github.com/termux/termux-packages.git 2>/dev/null || true
  git sparse-checkout set packages/postgresql packages/libandroid-shmem packages/libandroid-execinfo
  git fetch -q --depth 1 origin "$TERMUX_COMMIT" || fail "gagal fetch termux-packages@$TERMUX_COMMIT"
  git checkout -q FETCH_HEAD
  cd "$SRC_DIR/postgresql-${PG_VERSION}"
fi

PATCHES_APPLIED=""
shopt -s nullglob
for patch_file in "$TERMUX_DIR"/packages/postgresql/*.patch; do
  patch_name=$(basename "$patch_file")
  # Patch Termux memakai placeholder @TERMUX_PREFIX@ yang diganti build
  # system Termux; di sini diganti prefix placeholder WebForge.
  sed "s|@TERMUX_PREFIX@|${PG_PREFIX_PLACEHOLDER}|g" "$patch_file" > "$WORK_DIR/current.patch"
  if ! patch -p1 --dry-run < "$WORK_DIR/current.patch" >/dev/null; then
    fail "patch Termux gagal diterapkan: $patch_name (source PG $PG_VERSION vs termux@$TERMUX_COMMIT)"
  fi
  patch -p1 < "$WORK_DIR/current.patch" >/dev/null
  PATCHES_APPLIED="${PATCHES_APPLIED} ${patch_name}"
  log "patch diterapkan: $patch_name"
done
shopt -u nullglob
[ -n "$PATCHES_APPLIED" ] || fail "tidak ada patch Termux ditemukan di $TERMUX_DIR/packages/postgresql"
if grep -rq "@TERMUX_" src include 2>/dev/null; then
  log "peringatan: masih ada placeholder @TERMUX_ di source setelah patch:"
  grep -rl "@TERMUX_" src include | head -20 || true
fi

# --- Shim Termux (opsional, statis) ---
SHIM_LIBS=""
SHIM_STATUS="tidak dibangun"
# Patch Termux PostgreSQL menautkan -landroid-shmem; source shim ada di
# repo termux/libandroid-shmem (paket Termux hanya berisi build.sh).
SHMEM_VERSION="0.4"
SHMEM_TARBALL="$WORK_DIR/libandroid-shmem-${SHMEM_VERSION}.tar.gz"
if [ ! -f "$SHMEM_TARBALL" ]; then
  curl -fsSL --retry 3 -o "$SHMEM_TARBALL.tmp" "https://github.com/termux/libandroid-shmem/archive/v${SHMEM_VERSION}.tar.gz"
  mv "$SHMEM_TARBALL.tmp" "$SHMEM_TARBALL"
fi
SHMEM_SRC="$WORK_DIR/libandroid-shmem-${SHMEM_VERSION}"
if [ ! -d "$SHMEM_SRC" ]; then
  tar -xzf "$SHMEM_TARBALL" -C "$WORK_DIR"
fi
SHMEM_C=$(find "$SHMEM_SRC" -name "shmem.c" | head -1)
[ -n "$SHMEM_C" ] || fail "shmem.c tidak ditemukan di libandroid-shmem v${SHMEM_VERSION}"
log "build libandroid-shmem (statis)"
$CC -O2 -fPIC -std=gnu17 -D_GNU_SOURCE -D_PATH_TMP='"/data/local/tmp/"' \
  -include string.h -include fcntl.h \
  -I"$SHMEM_SRC" -c "$SHMEM_C" -o "$WORK_DIR/shmem.o"
$AR rcs "$PREFIX/lib/libandroid-shmem.a" "$WORK_DIR/shmem.o"
mkdir -p "$PREFIX/include/sys"
cp "$SHMEM_SRC/shm.h" "$PREFIX/include/sys/shm.h"
SHIM_LIBS="$SHIM_LIBS -landroid-shmem"
SHIM_STATUS="libandroid-shmem"
if [ -f "$TERMUX_DIR/packages/libandroid-execinfo/execinfo.c" ]; then
  log "build libandroid-execinfo (statis)"
  if $CC -O2 -fPIC -c "$TERMUX_DIR/packages/libandroid-execinfo/execinfo.c" -o "$WORK_DIR/execinfo.o"; then
    $AR rcs "$PREFIX/lib/libandroid-execinfo.a" "$WORK_DIR/execinfo.o"
    if [ -f "$TERMUX_DIR/packages/libandroid-execinfo/execinfo.h" ]; then
      cp "$TERMUX_DIR/packages/libandroid-execinfo/execinfo.h" "$PREFIX/include/"
    fi
    SHIM_LIBS="$SHIM_LIBS -landroid-execinfo"
    SHIM_STATUS="${SHIM_STATUS} libandroid-execinfo"
  else
    log "peringatan: libandroid-execinfo gagal dikompilasi; lanjut tanpa shim execinfo (backtrace nonaktif)"
  fi
fi
log "shim Android: $SHIM_STATUS"

# --- zic host (PostgreSQL cross-compile membutuhkan zic native) ---
HOST_ZIC="$WORK_DIR/host-build/src/timezone/zic"
if [ ! -x "$HOST_ZIC" ]; then
  log "build zic host"
  mkdir -p "$WORK_DIR/host-build"
  cd "$WORK_DIR/host-build"
  # Subshell tanpa variabel cross agar configure host bersih.
  (
    unset CC CXX AR RANLIB STRIP
    export CC=gcc
    # GCC host runner default C23; PG 16.3 membutuhkan mode C17.
    export CFLAGS="-std=gnu17 -O2"
    "$SRC_DIR/postgresql-${PG_VERSION}/configure" --without-readline --without-icu --disable-nls >/dev/null
    make -C src/timezone zic -j"$(nproc)"
  )
  cd "$SRC_DIR/postgresql-${PG_VERSION}"
fi
[ -x "$HOST_ZIC" ] || fail "zic host gagal dibangun"

# --- Configure + build target Android ---
log "configure PostgreSQL $PG_VERSION untuk aarch64-linux-android$ANDROID_API"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
export CFLAGS="-O2 -fPIC"
export CPPFLAGS="-I$PREFIX/include"
export LDFLAGS="-L$PREFIX/lib"
export LIBS="-lssl -lcrypto${SHIM_LIBS} -ldl"
./configure \
  --host=aarch64-linux-android \
  --build=x86_64-linux-gnu \
  --prefix="$PG_PREFIX_PLACEHOLDER" \
  --with-openssl \
  --without-readline \
  --without-libxml \
  --without-icu \
  --without-gssapi \
  --without-ldap \
  --without-pam \
  --without-perl \
  --without-python \
  --without-tcl \
  --without-lz4 \
  --without-zstd \
  --disable-nls \
  --enable-static \
  --disable-shared \
  USE_UNNAMED_POSIX_SEMAPHORES=1 \
  ZIC="$HOST_ZIC" \
  pgac_cv_prog_cc_LDFLAGS_EX_BE__Wl___export_dynamic=yes \
  pgac_cv_prog_cc_LDFLAGS__Wl___as_needed=yes \
  CC="$CC" CXX="$CXX" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP"

# Runtime memakai argv[0] symlink untuk menemukan share/; /proc/self/exe
# akan menembus symlink ke nativeLibraryDir dan merusak resolusi itu.
sed -i 's|^#define HAVE_PROC_SELF_EXE 1$|/* #undef HAVE_PROC_SELF_EXE (WebForge: resolusi argv0) */|' src/include/pg_config.h
# --disable-shared tidak cukup: libpq.so tetap terbuild dan initdb menautnya
# dinamis. Paksa enable_shared=no agar tool frontend menaut libpq.a statis.
sed -i 's/^enable_shared = yes/enable_shared = no/' src/Makefile.global
grep "^enable_shared" src/Makefile.global
grep -E "HAVE_PROC_SELF_EXE|USE_UNNAMED_POSIX_SEMAPHORES" src/include/pg_config.h || true

# Cek libpq-refs-stamp menolak referensi pthread_exit (sah di Bionic dan
# dipakai patch Termux); kecualikan pthread_exit dari grep cek tersebut.
sed -i 's/grep exit/grep -v pthread_exit | grep exit/' src/interfaces/libpq/Makefile

log "make PostgreSQL"
make -j"$(nproc)"

log "install ke staging"
rm -rf "$STAGE_DIR"
make install DESTDIR="$STAGE_DIR" >/dev/null
STAGED="$STAGE_DIR$PG_PREFIX_PLACEHOLDER"
[ -x "$STAGED/bin/postgres" ] || fail "postgres tidak terinstall di $STAGED/bin"

# --- Kemas binary sebagai lib*.so ---
package_binary() {
  local src_name="$1" out_name="$2" required="$3"
  if [ -f "$STAGED/bin/$src_name" ]; then
    cp "$STAGED/bin/$src_name" "$OUT_DIR/$out_name"
    $STRIP "$OUT_DIR/$out_name"
    log "kemas $src_name -> $out_name"
  elif [ "$required" = "required" ]; then
    fail "binary wajib tidak terbuild: $src_name"
  else
    log "peringatan: binary opsional tidak terbuild: $src_name"
  fi
}
package_binary postgres libpostgres.so required
package_binary initdb libinitdb.so required
package_binary pg_ctl libpgctl.so required
package_binary psql libpsql.so optional

# --- Kemas share (layout zip: share/postgresql/... di root zip) ---
rm -f "$OUT_DIR/postgres-share.zip"
(
  cd "$STAGED"
  zip -qr "$OUT_DIR/postgres-share.zip" share/postgresql
  if [ -d lib/postgresql ]; then
    zip -qr "$OUT_DIR/postgres-share.zip" lib/postgresql
  fi
)
[ -f "$STAGED/share/postgresql/postgres.bki" ] || fail "postgres.bki tidak ada di share hasil install"
[ -d "$STAGED/share/postgresql/timezone" ] || fail "share/timezone tidak ada di hasil install"

# --- Verifikasi ELF: AArch64, interpreter linker64, DT_NEEDED bionic saja ---
verify_elf() {
  local so="$1"
  file "$so"
  readelf -h "$so" | grep "AArch64" >/dev/null || fail "$so bukan ELF AArch64"
  readelf -l "$so" | grep -q "/system/bin/linker64" || fail "$so interpreter bukan /system/bin/linker64"
  local lib
  for lib in $(readelf -d "$so" | grep NEEDED | sed -E 's/.*\[(.*)\]/\1/'); do
    case "$lib" in
      libc.so|libm.so|libdl.so|libz.so|liblog.so) ;;
      *) fail "dependensi dinamis tak terduga pada $so: $lib (semua dependensi harus statis/bionic)" ;;
    esac
  done
  readelf -d "$so" | grep NEEDED || true
}
for so in "$OUT_DIR"/libpostgres.so "$OUT_DIR"/libinitdb.so "$OUT_DIR"/libpgctl.so; do
  verify_elf "$so"
done
[ -f "$OUT_DIR/libpsql.so" ] && verify_elf "$OUT_DIR/libpsql.so"

# --- Metadata versi ---
{
  echo "postgresql=$PG_VERSION"
  echo "postgresql_sha256=$PG_SHA256"
  echo "openssl=$OPENSSL_VERSION"
  echo "zlib=$ZLIB_VERSION"
  echo "android_api=$ANDROID_API"
  echo "target=aarch64-linux-android"
  echo "termux_commit=$TERMUX_COMMIT"
  echo "termux_patches=${PATCHES_APPLIED# }"
  echo "android_shims=$SHIM_STATUS"
  echo "prefix_placeholder=$PG_PREFIX_PLACEHOLDER"
  echo "built_utc=$(date -u +%FT%TZ)"
} > "$OUT_DIR/postgres-version.txt"

ls -lh "$OUT_DIR"
log "selesai: PostgreSQL $PG_VERSION -> $OUT_DIR"
