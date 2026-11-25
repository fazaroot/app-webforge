#!/usr/bin/env bash
# build-apache.sh — cross-compile Apache httpd untuk Android arm64
# (WebForge Mobile).
#
# Hasil: $OUTPUT_DIR/libhttpd.so (binary httpd dikemas sebagai .so agar sah
# dieksekusi dari nativeLibraryDir; patuh W^X Android). Rantai dependensi
# dibangun statis dari source dengan clang NDK aarch64-linux-android<API>:
# expat, PCRE2, OpenSSL, APR, APR-util, lalu httpd dengan modul inti
# statis. DT_NEEDED hasil akhir hanya pustaka Bionic
# (libc/libm/libdl/liblog).
#
# Environment yang diterima:
#   ANDROID_NDK_HOME Path NDK. Bila kosong dicari dari $ANDROID_HOME/ndk/*.
#   ANDROID_API      API level target (default 26).
#   OUTPUT_DIR       Folder hasil (default $PWD/native-out).
#   WORK_DIR         Folder kerja (default $PWD/native-work/apache);
#                    prefix dependensi statis ada di $WORK_DIR/prefix.
#   HTTPD_VERSION / APR_VERSION / APR_UTIL_VERSION / PCRE2_VERSION /
#   EXPAT_VERSION / OPENSSL_VERSION
#                    Override versi (default pinned di bawah).
#
# Modul statis: mime, dir, log_config, authz_core, authz_host,
# authn_core, authz_user, unixd, setenvif, alias, rewrite, env, headers,
# reqtimeout, ssl, socache_shmcb, watchdog. MPM event, mod_so dimatikan
# (tidak ada modul dinamis/LoadModule).
#
# mod_proxy SENGAJA tidak dikompilasi pada v0.3.0: reverse proxy Apache
# tidak didukung build ini, dan core (webservers.rs) mengembalikan error
# jujur bila proxy diminta untuk Apache. mod_ssl dikompilasi agar
# custom_config pengguna bisa memakai HTTPS; config hasil generate core
# tetap HTTP polos.
#
# Uji cross-compile: APR/httpd memakai autoconf. Uji yang menjalankan
# program target diganti cache variables ac_cv_*/apr_cv_* di bawah
# (ukuran tipe LP64 aarch64, /dev/zero, setpgrp, epoll, mutex, mmap).
# gen_test_char (program host yang menghasilkan test_char.h) dikompilasi
# dengan CC_FOR_BUILD=cc — dukungan resmi httpd untuk cross-compile.
set -euo pipefail

HTTPD_VERSION="${HTTPD_VERSION:-2.4.62}"
APR_VERSION="${APR_VERSION:-1.7.5}"
APR_UTIL_VERSION="${APR_UTIL_VERSION:-1.6.3}"
PCRE2_VERSION="${PCRE2_VERSION:-10.44}"
EXPAT_VERSION="${EXPAT_VERSION:-2.6.4}"
OPENSSL_VERSION="${OPENSSL_VERSION:-3.3.2}"
ANDROID_API="${ANDROID_API:-26}"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/native-out}"
WORK_DIR="${WORK_DIR:-$PWD/native-work/apache}"

echo "== Build Apache httpd ${HTTPD_VERSION} (android/arm64, API ${ANDROID_API}) =="

# --- Toolchain NDK ---------------------------------------------------------
if [ -z "${ANDROID_NDK_HOME:-}" ] && [ -n "${ANDROID_HOME:-}" ]; then
  ANDROID_NDK_HOME=$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1 || true)
fi
if [ -z "${ANDROID_NDK_HOME:-}" ] || [ ! -d "$ANDROID_NDK_HOME" ]; then
  echo "::error::Android NDK tidak ditemukan. Set ANDROID_NDK_HOME atau ANDROID_HOME." >&2
  exit 1
fi
TOOLCHAIN=$(ls -d "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/* 2>/dev/null | head -1 || true)
if [ -z "$TOOLCHAIN" ] || [ ! -d "$TOOLCHAIN/bin" ]; then
  echo "::error::Toolchain LLVM NDK tidak ditemukan di $ANDROID_NDK_HOME/toolchains/llvm/prebuilt" >&2
  exit 1
fi
CC="$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang"
CXX="$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang++"
AR="$TOOLCHAIN/bin/llvm-ar"
RANLIB="$TOOLCHAIN/bin/llvm-ranlib"
STRIP="$TOOLCHAIN/bin/llvm-strip"
if command -v readelf >/dev/null 2>&1; then
  READELF="readelf"
else
  READELF="$TOOLCHAIN/bin/llvm-readelf"
fi
if [ ! -x "$CC" ]; then
  echo "::error::Compiler NDK tidak ditemukan: $CC" >&2
  exit 1
fi
"$CC" --version | head -1
export PATH="$TOOLCHAIN/bin:$PATH"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"

JOBS=$(nproc 2>/dev/null || echo 2)
PREFIX="$WORK_DIR/prefix"
SRC="$WORK_DIR/src"
DL="$WORK_DIR/downloads"
mkdir -p "$OUTPUT_DIR" "$PREFIX" "$SRC" "$DL"

download() {
  local url="$1" out="$2"
  if [ ! -f "$out" ]; then
    echo "-- Unduh $url --"
    curl -fsSL --retry 3 -o "$out.tmp" "$url"
    mv "$out.tmp" "$out"
  fi
}

# Cache variables cross-compile bersama (LP64 aarch64, kernel Linux).
# Dipakai APR dan httpd agar configure tidak menjalankan program target.
CROSS_CACHE_VARS=(
  ac_cv_file__dev_zero=yes
  ac_cv_func_setpgrp_void=yes
  ac_cv_sizeof_char=1
  ac_cv_sizeof_short=2
  ac_cv_sizeof_int=4
  ac_cv_sizeof_long=8
  ac_cv_sizeof_long_long=8
  ac_cv_sizeof_void_p=8
  ac_cv_sizeof_size_t=8
  ac_cv_sizeof_ssize_t=8
  ac_cv_sizeof_off_t=8
  ac_cv_sizeof_pid_t=4
  ac_cv_sizeof_struct_iovec=16
  apr_cv_sizeof_char=1
  apr_cv_sizeof_short=2
  apr_cv_sizeof_int=4
  apr_cv_sizeof_long=8
  apr_cv_sizeof_long_long=8
  apr_cv_sizeof_void_p=8
  apr_cv_sizeof_size_t=8
  apr_cv_sizeof_ssize_t=8
  apr_cv_sizeof_off_t=8
  apr_cv_sizeof_pid_t=4
  apu_cv_sizeof_char=1
  apu_cv_sizeof_short=2
  apu_cv_sizeof_int=4
  apu_cv_sizeof_long=8
  apu_cv_sizeof_long_long=8
  apu_cv_sizeof_void_p=8
  apu_cv_sizeof_size_t=8
  apu_cv_sizeof_ssize_t=8
  apu_cv_sizeof_off_t=8
  apu_cv_sizeof_pid_t=4
)

# --- expat (statis, via CMake + toolchain NDK) ------------------------------
echo "-- Build expat ${EXPAT_VERSION} (statis) --"
EXPAT_TAG="R_$(echo "$EXPAT_VERSION" | tr '.' '_')"
download "https://github.com/libexpat/libexpat/releases/download/${EXPAT_TAG}/expat-${EXPAT_VERSION}.tar.gz" "$DL/expat.tar.gz"
rm -rf "$SRC/expat-${EXPAT_VERSION}" "$SRC/expat-build"
tar -xzf "$DL/expat.tar.gz" -C "$SRC"
cmake -S "$SRC/expat-${EXPAT_VERSION}" -B "$SRC/expat-build" \
  -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a \
  -DANDROID_PLATFORM="android-${ANDROID_API}" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DEXPAT_SHARED_LIBS=OFF \
  -DEXPAT_BUILD_TOOLS=OFF \
  -DEXPAT_BUILD_EXAMPLES=OFF \
  -DEXPAT_BUILD_TESTS=OFF \
  -DEXPAT_BUILD_DOCS=OFF
cmake --build "$SRC/expat-build" -j"$JOBS"
cmake --install "$SRC/expat-build"

# --- PCRE2 (statis, tanpa JIT) ----------------------------------------------
echo "-- Build PCRE2 ${PCRE2_VERSION} (statis) --"
download "https://github.com/PCRE2Project/pcre2/releases/download/pcre2-${PCRE2_VERSION}/pcre2-${PCRE2_VERSION}.tar.gz" "$DL/pcre2.tar.gz"
rm -rf "$SRC/pcre2-${PCRE2_VERSION}"
tar -xzf "$DL/pcre2.tar.gz" -C "$SRC"
(
  cd "$SRC/pcre2-${PCRE2_VERSION}"
  ./configure \
    --host=aarch64-linux-android \
    --build=x86_64-linux-gnu \
    --prefix="$PREFIX" \
    --enable-static \
    --disable-shared \
    --disable-jit \
    --enable-unicode \
    CC="$CC" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP"
  make -j"$JOBS"
  make install
)

# --- OpenSSL (statis) --------------------------------------------------------
echo "-- Build OpenSSL ${OPENSSL_VERSION} (statis) --"
download "https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VERSION}/openssl-${OPENSSL_VERSION}.tar.gz" "$DL/openssl.tar.gz"
rm -rf "$SRC/openssl-${OPENSSL_VERSION}"
tar -xzf "$DL/openssl.tar.gz" -C "$SRC"
(
  cd "$SRC/openssl-${OPENSSL_VERSION}"
  ./Configure android-arm64 "-D__ANDROID_API__=${ANDROID_API}" \
    no-shared no-tests no-docs \
    --prefix="$PREFIX" --openssldir="$PREFIX/ssl"
  make -j"$JOBS" build_sw
  make install_sw
)

# --- APR (statis) -------------------------------------------------------------
echo "-- Build APR ${APR_VERSION} (statis) --"
download "https://archive.apache.org/dist/apr/apr-${APR_VERSION}.tar.gz" "$DL/apr.tar.gz"
rm -rf "$SRC/apr-${APR_VERSION}"
tar -xzf "$DL/apr.tar.gz" -C "$SRC"
(
  cd "$SRC/apr-${APR_VERSION}"
  ./configure \
    --host=aarch64-linux-android \
    --build=x86_64-linux-gnu \
    --prefix="$PREFIX" \
    --enable-static \
    --disable-shared \
    CC="$CC" CXX="$CXX" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP" \
    "${CROSS_CACHE_VARS[@]}" \
    apr_cv_process_shared_works=yes \
    apr_cv_mutex_robust_shared=no \
    apr_cv_tcp_nodelay_with_cork=yes \
    apr_cv_epoll=yes \
    ac_cv_func_poll=yes \
    ac_cv_func_mmap_fixed_mapped=yes
  make -j"$JOBS"
  make install
)

# --- APR-util (statis) ---------------------------------------------------------
echo "-- Build APR-util ${APR_UTIL_VERSION} (statis) --"
download "https://archive.apache.org/dist/apr/apr-util-${APR_UTIL_VERSION}.tar.gz" "$DL/apr-util.tar.gz"
rm -rf "$SRC/apr-util-${APR_UTIL_VERSION}"
tar -xzf "$DL/apr-util.tar.gz" -C "$SRC"
(
  cd "$SRC/apr-util-${APR_UTIL_VERSION}"
  ./configure \
    --host=aarch64-linux-android \
    --build=x86_64-linux-gnu \
    --prefix="$PREFIX" \
    --enable-static \
    --disable-shared \
    --with-apr="$PREFIX" \
    --with-expat="$PREFIX" \
    --without-crypto \
    --without-ldap \
    CC="$CC" CXX="$CXX" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP" \
    "${CROSS_CACHE_VARS[@]}"
  make -j"$JOBS"
  make install
)

# --- httpd ---------------------------------------------------------------------
echo "-- Build httpd ${HTTPD_VERSION} --"
download "https://archive.apache.org/dist/httpd/httpd-${HTTPD_VERSION}.tar.gz" "$DL/httpd.tar.gz"
rm -rf "$SRC/httpd-${HTTPD_VERSION}"
tar -xzf "$DL/httpd.tar.gz" -C "$SRC"
(
  cd "$SRC/httpd-${HTTPD_VERSION}"
  # Prefix compile-time hanya placeholder; runtime memakai ServerRoot
  # server home dari config hasil generate core (-d + ServerRoot).
  CC_FOR_BUILD=cc ./configure \
    --host=aarch64-linux-android \
    --build=x86_64-linux-gnu \
    --prefix=/webforge/httpd \
    --with-apr="$PREFIX" \
    --with-apr-util="$PREFIX" \
    --with-pcre="$PREFIX/bin/pcre2-config" \
    --with-ssl="$PREFIX" \
    --with-expat="$PREFIX" \
    --with-mpm=event \
    --enable-mods-static="mime dir log_config authz_core authz_host authn_core authz_user unixd setenvif alias rewrite env headers reqtimeout ssl socache_shmcb watchdog" \
    --enable-mods-shared=none \
    --disable-so \
    CC="$CC" CXX="$CXX" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP" \
    "${CROSS_CACHE_VARS[@]}" \
    ac_cv_func_poll=yes
  make -j"$JOBS"
)

# Binary httpd bisa berupa wrapper libtool (skrip) dengan ELF asli di
# .libs/httpd; ambil ELF aslinya.
HTTPD_BIN=""
for candidate in \
  "$SRC/httpd-${HTTPD_VERSION}/.libs/httpd" \
  "$SRC/httpd-${HTTPD_VERSION}/httpd"; do
  if [ -f "$candidate" ] && [ "$(head -c 2 "$candidate")" != "#!" ]; then
    HTTPD_BIN="$candidate"
    break
  fi
done
if [ -z "$HTTPD_BIN" ]; then
  echo "::error::Binary httpd hasil build tidak ditemukan di $SRC/httpd-${HTTPD_VERSION}" >&2
  exit 1
fi

OUT="$OUTPUT_DIR/libhttpd.so"
cp "$HTTPD_BIN" "$OUT"
"$STRIP" "$OUT"
chmod +x "$OUT"

echo "-- Verifikasi ELF + DT_NEEDED --"
if command -v file >/dev/null 2>&1; then
  file "$OUT"
fi
"$READELF" -h "$OUT" | grep -E "Class|Type|Machine"
if ! "$READELF" -h "$OUT" | grep -q "AArch64"; then
  echo "::error::libhttpd.so bukan ELF AArch64." >&2
  exit 1
fi
"$READELF" -d "$OUT" | grep NEEDED || echo "(tidak ada DT_NEEDED)"
for lib in $("$READELF" -d "$OUT" | grep NEEDED | sed -E 's/.*\[(.*)\]/\1/' || true); do
  case "$lib" in
    libc.so|libm.so|libdl.so|liblog.so) ;;
    *)
      echo "::error::Dependensi dinamis tak terduga pada libhttpd.so: $lib (dependensi harus statis)" >&2
      exit 1
      ;;
  esac
done

SIZE=$(stat -c%s "$OUT" 2>/dev/null || wc -c < "$OUT")
echo "Selesai: $OUT ($SIZE bytes)"
ls -lh "$OUT"
