#!/usr/bin/env bash
# WebForge Mobile — cross-compile MariaDB 10.11 untuk Android arm64 (API 26).
#
# Output (di $OUT_DIR):
#   libmariadbd.so          (server; satu-satunya binary ELF yang wajib)
#   mariadb-install-db.sh   (skrip shell bawaan MariaDB; BUKAN ELF — lihat bawah)
#   mariadb-share.zip       (isi root zip: charsets/, english/errmsg.sys, *.sql)
#   mariadb-version.txt
#
# Kenapa tidak ada libmariadb-install-db.so: mariadb-install-db di MariaDB
# 10.11 adalah skrip shell yang memanggil `mariadbd --bootstrap`. Skrip
# shell tidak bisa dikemas sebagai .so dan tidak andal di Android (tidak
# ada perl/coreutils lengkap). Jalur inisialisasi resmi WebForge adalah
# `mariadbd --bootstrap` langsung dengan file SQL sistem dari share zip
# (core/src/db_runtime.rs: mariadb_bootstrap_args +
# mariadb_bootstrap_sql_files). Skrip install-db tetap dikemas sebagai
# referensi/diagnostik.
#
# Cross-compile MariaDB punya dua kebutuhan khusus yang ditangani script ini:
# 1. Host tools (comp_err, gen_lex_hash, comp_sql, gen_lex_token) harus
#    dibangun native lebih dulu dan diteruskan lewat IMPORT_EXECUTABLES;
#    tanpa itu build target gagal saat menggenerate errmsg.sys/parser.
# 2. Beberapa deteksi CMake MariaDB memakai try_run (program uji yang
#    tidak bisa dijalankan saat cross-compile). Nilai preset untuk
#    aarch64 diberikan eksplisit (STACK_DIRECTION, atomic builtins).
#    Bila CI menemukan try_run lain yang gagal, tambahkan preset-nya di
#    sini dengan nilai yang benar untuk aarch64 — jangan menebak.
#
# Keputusan build:
# - Plugin minimal & statis: auth_socket, PAM, CONNECT, RocksDB, Spider,
#   Mroonga, OQGraph, Sphinx, Federated, Archive, Blackhole, TokuDB,
#   Performance Schema dimatikan. Auth memakai mysql_native_password
#   bawaan server. Storage engine: InnoDB, Aria, MyISAM, MEMORY, CSV.
# - Tanpa systemd, tanpa unit test, tanpa embedded server, tanpa client
#   (core mengakses MariaDB lewat sqlx, bukan binary client).
# - OpenSSL + zlib statis dari source; PCRE2 bundled MariaDB; STL C++
#   statis (ANDROID_STL=c++_static) agar DT_NEEDED hanya bionic.
# - InnoDB native AIO tidak tersedia di Android (tanpa libaio); runtime
#   mematikannya eksplisit di my.cnf (innodb_use_native_aio=OFF).
#
# Patch Termux TIDAK diterapkan secara default: paket mariadb Termux saat
# riset ini dibuat sudah di seri 13.x, patch-nya tidak terbukti cocok
# untuk 10.11.8. Set APPLY_TERMUX_PATCHES=1 hanya untuk eksperimen; script
# akan gagal keras bila patch tidak cocok.
#
# Script ini TIDAK pernah dijalankan di sandbox lokal (tidak ada NDK);
# bukti pertama adalah run CI. Blokir yang mungkin muncul di run pertama
# dan titik persisnya dicatat di laporan kelompok DATABASE.
set -euo pipefail

MARIADB_VERSION="${MARIADB_VERSION:-10.11.8}"
# MD5 dari halaman paket MariaDB 10.11.8 (Linux From Scratch); SHA256
# hasil unduhan dicatat ke mariadb-version.txt untuk pin berikutnya.
MARIADB_MD5="${MARIADB_MD5:-ba7abfae7947893c5a5343180808b0cb}"
OPENSSL_VERSION="${OPENSSL_VERSION:-3.3.2}"
ZLIB_VERSION="${ZLIB_VERSION:-1.3.1}"
ANDROID_API="${ANDROID_API:-26}"
APPLY_TERMUX_PATCHES="${APPLY_TERMUX_PATCHES:-0}"
TERMUX_COMMIT="${TERMUX_COMMIT:-ff9d0d08b7274dd2923b90e6c6437aa5d0cc573d}"

WORK_DIR="${WORK_DIR:-$PWD/db-build/mariadb}"
OUT_DIR="${OUT_DIR:-$PWD/mariadb-out}"
PREFIX="$WORK_DIR/prefix"
SRC_DIR="$WORK_DIR/src"
HOST_BUILD="$WORK_DIR/host-build"
TARGET_BUILD="$WORK_DIR/target-build"
STAGE_SHARE="$WORK_DIR/stage-share/mysql"

log() { echo "[build-mariadb] $*"; }
fail() { echo "::error::[build-mariadb] $*" >&2; exit 1; }

mkdir -p "$WORK_DIR" "$OUT_DIR" "$PREFIX/lib/pkgconfig" "$PREFIX/include" "$SRC_DIR"

for tool in curl tar cmake make zip readelf file md5sum sha256sum; do
  command -v "$tool" >/dev/null || fail "tool host tidak ditemukan: $tool"
done
command -v bison >/dev/null || log "peringatan: bison tidak ada; tarball rilis MariaDB menyertakan parser hasil generate, tetapi host build bisa membutuhkannya"

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

# --- zlib statis ---
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

# --- Source MariaDB ---
cd "$SRC_DIR"
if [ ! -d "mariadb-${MARIADB_VERSION}" ]; then
  log "unduh MariaDB $MARIADB_VERSION"
  curl -fsSL -o mariadb.tar.gz "https://downloads.mariadb.org/interstitial/mariadb-${MARIADB_VERSION}/source/mariadb-${MARIADB_VERSION}.tar.gz"
  echo "${MARIADB_MD5}  mariadb.tar.gz" | md5sum -c - || fail "MD5 MariaDB tidak cocok"
  tar xf mariadb.tar.gz
fi
MARIADB_SHA256=$(sha256sum mariadb.tar.gz | awk '{print $1}')
MARIADB_SRC="$SRC_DIR/mariadb-${MARIADB_VERSION}"
cd "$MARIADB_SRC"

# --- Patch Termux (opsional, default mati — lihat header) ---
PATCHES_APPLIED=""
if [ "$APPLY_TERMUX_PATCHES" = "1" ]; then
  TERMUX_DIR="$WORK_DIR/termux-packages"
  if [ ! -d "$TERMUX_DIR/packages/mariadb" ]; then
    log "ambil patch Termux @ $TERMUX_COMMIT"
    mkdir -p "$TERMUX_DIR"
    cd "$TERMUX_DIR"
    git init -q
    git remote add origin https://github.com/termux/termux-packages.git 2>/dev/null || true
    git sparse-checkout set packages/mariadb
    git fetch -q --depth 1 origin "$TERMUX_COMMIT" || fail "gagal fetch termux-packages@$TERMUX_COMMIT"
    git checkout -q FETCH_HEAD
    cd "$MARIADB_SRC"
  fi
  shopt -s nullglob
  for patch_file in "$TERMUX_DIR"/packages/mariadb/*.patch; do
    patch_name=$(basename "$patch_file")
    sed "s|@TERMUX_PREFIX@|/webforge/mariadb|g" "$patch_file" > "$WORK_DIR/current.patch"
    if patch -p1 -R --dry-run < "$WORK_DIR/current.patch" >/dev/null 2>&1; then
      log "patch sudah diterapkan (cache): $patch_name"
    elif patch -p1 --dry-run < "$WORK_DIR/current.patch" >/dev/null; then
      patch -p1 < "$WORK_DIR/current.patch" >/dev/null
    else
      fail "patch Termux gagal diterapkan: $patch_name (patch seri Termux saat ini tidak terbukti untuk MariaDB $MARIADB_VERSION)"
    fi
    PATCHES_APPLIED="${PATCHES_APPLIED} ${patch_name}"
    log "patch diterapkan: $patch_name"
  done
  shopt -u nullglob
fi

# Opsi CMake bersama host & target.
COMMON_CMAKE_ARGS=(
  -DCMAKE_BUILD_TYPE=Release
  -DWITH_UNIT_TESTS=OFF
  -DWITH_EMBEDDED_SERVER=OFF
  -DWITH_SYSTEMD=no
  -DWITH_PCRE=bundled
  -DPLUGIN_AUTH_SOCKET=NO
  -DPLUGIN_AUTH_PAM=NO
  -DPLUGIN_AUTH_ED25519=NO
  -DPLUGIN_CONNECT=NO
  -DPLUGIN_TOKUDB=NO
  -DPLUGIN_ROCKSDB=NO
  -DPLUGIN_SPIDER=NO
  -DPLUGIN_MROONGA=NO
  -DPLUGIN_OQGRAPH=NO
  -DPLUGIN_SPHINX=NO
  -DPLUGIN_FEDERATED=NO
  -DPLUGIN_ARCHIVE=NO
  -DPLUGIN_BLACKHOLE=NO
  -DPLUGIN_PERFSCHEMA=NO
)

# --- Host build: hanya tool generator untuk IMPORT_EXECUTABLES ---
if [ ! -f "$HOST_BUILD/import_executables.cmake" ]; then
  log "host build untuk IMPORT_EXECUTABLES"
  (
    unset AR RANLIB STRIP
    export CC=gcc
    export CXX=g++
    cmake -S "$MARIADB_SRC" -B "$HOST_BUILD" -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
      -DCMAKE_C_FLAGS="-std=gnu17 -Wno-error=incompatible-pointer-types -Wno-error=implicit-function-declaration" \
      -DCMAKE_CXX_FLAGS="-Wno-error=incompatible-pointer-types" \
      "${COMMON_CMAKE_ARGS[@]}" \
      -DWITH_SSL=system \
      -DWITH_ZLIB=bundled
    cmake --build "$HOST_BUILD" -j"$(nproc)" --target comp_err gen_lex_hash comp_sql gen_lex_token
  )
fi
[ -f "$HOST_BUILD/import_executables.cmake" ] || fail "import_executables.cmake tidak dihasilkan host build"

# --- ncurses statis untuk readline klien (configure mewajibkan Curses) ---
if [ ! -f "$PREFIX/lib/libncurses.a" ]; then
  log "build ncurses 6.5 (statis)"
  if [ ! -f "$WORK_DIR/ncurses.tar.gz" ]; then
    curl -fsSL --retry 3 -o "$WORK_DIR/ncurses.tar.gz.tmp" "https://ftp.gnu.org/pub/gnu/ncurses/ncurses-6.5.tar.gz"
    mv "$WORK_DIR/ncurses.tar.gz.tmp" "$WORK_DIR/ncurses.tar.gz"
  fi
  rm -rf "$WORK_DIR/ncurses-6.5"
  tar -xzf "$WORK_DIR/ncurses.tar.gz" -C "$WORK_DIR"
  (
    cd "$WORK_DIR/ncurses-6.5"
    CC="$CC" BUILD_CC=gcc ./configure \
      --host=aarch64-linux-android \
      --prefix="$PREFIX" \
      --without-ada --without-cxx --without-progs --without-tests \
      --without-manpages --disable-db-install \
      --enable-static --disable-shared --disable-widec \
      --with-normal --without-debug --enable-overwrite \
      AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP"
    make -j"$(nproc)"
    make install.libs install.includes
  )
fi
[ -f "$PREFIX/lib/libncurses.a" ] || fail "libncurses.a tidak terbuild"

# --- Target build Android ---
log "configure MariaDB $MARIADB_VERSION untuk aarch64-linux-android$ANDROID_API"
cmake -S "$MARIADB_SRC" -B "$TARGET_BUILD" -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
  -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a \
  -DANDROID_PLATFORM="android-${ANDROID_API}" \
  -DANDROID_STL=c++_static \
  -DCMAKE_PREFIX_PATH="$PREFIX" \
  -DCMAKE_C_FLAGS="-include ctype.h -Wno-error=implicit-function-declaration -Wno-error=incompatible-pointer-types" \
  -DCMAKE_CXX_FLAGS="-Wno-error=implicit-function-declaration -Wno-error=incompatible-pointer-types" \
  -DCURSES_LIBRARY="$PREFIX/lib/libncurses.a" \
  -DCURSES_INCLUDE_PATH="$PREFIX/include" \
  -DIMPORT_EXECUTABLES="$HOST_BUILD/import_executables.cmake" \
  -DWITH_SSL=system \
  -DCMAKE_FIND_ROOT_PATH="$PREFIX" \
  -DOPENSSL_ROOT_DIR="$PREFIX" \
  -DOPENSSL_INCLUDE_DIR="$PREFIX/include" \
  -DOPENSSL_SSL_LIBRARY="$PREFIX/lib/libssl.a" \
  -DOPENSSL_CRYPTO_LIBRARY="$PREFIX/lib/libcrypto.a" \
  -DOPENSSL_USE_STATIC_LIBS=TRUE \
  -DHAVE_LIBRESSL=FALSE \
  -DLIBRESSL_RESULT=1 \
  -DWITH_ZLIB=system \
  -DWITH_WSREP=OFF \
  -DZLIB_ROOT="$PREFIX" \
  -DSTACK_DIRECTION=-1 \
  -DHAVE_GCC_ATOMIC_BUILTINS_EXITCODE=0 \
  -DHAVE_GCC_ATOMIC_BUILTINS_EXITCODE__TRYRUN_OUTPUT="" \
  -DHAVE_GCC_SYNC_BUILTINS_EXITCODE=0 \
  -DHAVE_GCC_SYNC_BUILTINS_EXITCODE__TRYRUN_OUTPUT="" \
  -DHAVE_LLVM_LIBCPP_EXITCODE=0 \
  -DHAVE_LLVM_LIBCPP_EXITCODE__TRYRUN_OUTPUT="" \
  "${COMMON_CMAKE_ARGS[@]}"

log "build mariadbd"
cmake --build "$TARGET_BUILD" -j"$(nproc)" --target mariadbd

MARIADBD_BIN="$TARGET_BUILD/sql/mariadbd"
[ -f "$MARIADBD_BIN" ] || fail "mariadbd tidak terbuild di $MARIADBD_BIN"
cp "$MARIADBD_BIN" "$OUT_DIR/libmariadbd.so"
$STRIP "$OUT_DIR/libmariadbd.so"

# mariadb-install-db adalah skrip shell hasil generate CMake; kemas apa
# adanya sebagai referensi. Jalur utama Android adalah --bootstrap.
if [ -f "$TARGET_BUILD/scripts/mariadb-install-db" ]; then
  cp "$TARGET_BUILD/scripts/mariadb-install-db" "$OUT_DIR/mariadb-install-db.sh"
  log "kemas mariadb-install-db.sh (skrip shell, bukan ELF)"
else
  log "peringatan: scripts/mariadb-install-db tidak dihasilkan build"
fi

# --- Kemas share: charsets + errmsg.sys + SQL sistem untuk bootstrap ---
log "kemas mariadb-share.zip"
rm -rf "$WORK_DIR/stage-share"
mkdir -p "$STAGE_SHARE/charsets" "$STAGE_SHARE/english"
cp "$MARIADB_SRC"/strings/charsets/*.xml "$STAGE_SHARE/charsets/" 2>/dev/null || true
[ -f "$STAGE_SHARE/charsets/Index.xml" ] || fail "charsets/Index.xml tidak ditemukan di source MariaDB"
ERRMSG_SYS=$(find "$TARGET_BUILD" -path "*english/errmsg.sys" | head -1)
[ -n "$ERRMSG_SYS" ] || fail "english/errmsg.sys tidak dihasilkan build (comp_err host gagal?)"
cp "$ERRMSG_SYS" "$STAGE_SHARE/english/errmsg.sys"
for sql in mysql_system_tables.sql mysql_system_tables_data.sql mysql_performance_tables.sql mysql_sys_schema.sql fill_help_tables.sql; do
  [ -f "$MARIADB_SRC/sql/share/$sql" ] || fail "SQL sistem tidak ditemukan: sql/share/$sql"
  cp "$MARIADB_SRC/sql/share/$sql" "$STAGE_SHARE/$sql"
done
rm -f "$OUT_DIR/mariadb-share.zip"
(cd "$STAGE_SHARE" && zip -qr "$OUT_DIR/mariadb-share.zip" .)

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
      *) fail "dependensi dinamis tak terduga pada $so: $lib (semua dependensi harus statis/bionic; periksa ANDROID_STL dan WITH_SSL)" ;;
    esac
  done
  readelf -d "$so" | grep NEEDED || true
}
verify_elf "$OUT_DIR/libmariadbd.so"

# --- Metadata versi ---
{
  echo "mariadb=$MARIADB_VERSION"
  echo "mariadb_md5=$MARIADB_MD5"
  echo "mariadb_sha256=$MARIADB_SHA256"
  echo "openssl=$OPENSSL_VERSION"
  echo "zlib=$ZLIB_VERSION"
  echo "android_api=$ANDROID_API"
  echo "target=aarch64-linux-android"
  echo "termux_patches=${PATCHES_APPLIED# }"
  echo "install_db=bukan ELF; inisialisasi via mariadbd --bootstrap"
  echo "built_utc=$(date -u +%FT%TZ)"
} > "$OUT_DIR/mariadb-version.txt"

ls -lh "$OUT_DIR"
log "selesai: MariaDB $MARIADB_VERSION -> $OUT_DIR"
