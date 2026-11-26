#!/usr/bin/env bash
# build-php.sh — cross-compile PHP CLI + PHP-FPM untuk Android arm64
# (WebForge Mobile v0.5.0).
#
# Hasil di $OUTPUT_DIR:
#   libphp<82|83|84>.so       binary PHP CLI (sapi/cli/php)
#   libphpfpm<82|83|84>.so    binary PHP-FPM (sapi/fpm/php-fpm)
#   icudt74l.dat              data ICU 74.2 (packaging archive) untuk intl
# Binary dikemas sebagai .so agar sah dieksekusi dari nativeLibraryDir
# (patuh W^X Android). DT_NEEDED hasil akhir hanya pustaka Bionic
# (libc/libm/libdl/libz/liblog); semua dependensi lain statis.
#
# Environment yang diterima:
#   PHP_VERSION      Versi PHP persis, mis. 8.3.37 (wajib cocok seri).
#   PHP_SERIES       Seri 8.2 | 8.3 | 8.4. Bila kosong diturunkan dari
#                    PHP_VERSION. Suffix nama file: 82 | 83 | 84.
#   ANDROID_NDK_HOME Path NDK. Bila kosong dicari dari $ANDROID_HOME/ndk/*.
#   ANDROID_API      API level target (default 26).
#   OUTPUT_DIR       Folder hasil (default $PWD/php-out).
#   WORK_DIR         Folder kerja (default $PWD/php-work/<suffix>);
#                    prefix dependensi statis ada di $WORK_DIR/prefix.
#   *_VERSION        Override versi dependensi (default pinned di bawah).
#
# Ekstensi yang dibangun (statis, --disable-all lalu diaktifkan satu per
# satu): bcmath ctype dom fileinfo filter hash json libxml mbstring
# openssl pdo pdo_sqlite phar posix session simplexml sockets sqlite3
# tokenizer xml xmlreader xmlwriter zip zlib curl gd exif soap ftp
# pcntl intl. hash/json selalu terkompilasi di PHP 8 (tidak ada flag
# --enable- terpisah). Verifikasi di akhir skrip memeriksa simbol
# phpext_<ext>_ptr untuk SETIAP ekstensi di atas pada binary CLI dan
# FPM (sebelum strip, via llvm-nm/nm) dan gagal keras bila ada yang
# hilang — jangan mengurangi daftar verifikasi diam-diam; bila sebuah
# ekstensi terbukti tidak bisa dibangun, laporkan apa adanya.
#
# Dependensi statis dari source: zlib, SQLite (amalgamation), libxml2
# (CMake, tanpa iconv), OpenSSL, Oniguruma, libzip, libpng,
# libjpeg-turbo, freetype, libcurl (OpenSSL+zlib), ICU 74.2 (host
# cross-build untuk tools ICU, data packaging archive).
#
# Catatan Bionic (warisan build PHP 8.3.14 lama):
# - configure mendeteksi simbol res_n* ada, tetapi struct __res_state
#   tidak utuh untuk aplikasi sehingga ext/standard/dns.c gagal
#   kompilasi. Deteksi itu dimatikan di php_config.h; fungsi DNS PHP
#   memakai jalur fallback.
# - Bionic tidak mendeklarasikan getdtablesize(); PHP memanggilnya
#   tanpa guard. Diganti sysconf(_SC_OPEN_MAX) yang setara.
# - ICU/intl adalah C++: libc++ ditautkan STATIS (libc++_static +
#   libc++abi + libc++demangle bila ada di sysroot NDK) agar DT_NEEDED
#   tetap hanya Bionic. Jangan ganti dengan libc++_shared.so.
set -euo pipefail

PHP_VERSION="${PHP_VERSION:-8.3.37}"
PHP_SERIES="${PHP_SERIES:-$(echo "$PHP_VERSION" | cut -d. -f1,2)}"
case "$PHP_SERIES" in
  8.2) SUFFIX="82" ;;
  8.3) SUFFIX="83" ;;
  8.4) SUFFIX="84" ;;
  *)
    echo "::error::PHP_SERIES tidak didukung: $PHP_SERIES (harus 8.2, 8.3, atau 8.4)" >&2
    exit 1
    ;;
esac
case "$PHP_VERSION" in
  "$PHP_SERIES".*) ;;
  *)
    echo "::error::PHP_VERSION ($PHP_VERSION) tidak cocok dengan PHP_SERIES ($PHP_SERIES)" >&2
    exit 1
    ;;
esac

LIBXML2_VERSION="${LIBXML2_VERSION:-2.12.9}"
OPENSSL_VERSION="${OPENSSL_VERSION:-3.3.2}"
ONIGURUMA_VERSION="${ONIGURUMA_VERSION:-6.9.9}"
SQLITE_VERSION="${SQLITE_VERSION:-3.46.1}"
SQLITE_AMALGAMATION="${SQLITE_AMALGAMATION:-3460100}"
LIBZIP_VERSION="${LIBZIP_VERSION:-1.10.1}"
ZLIB_VERSION="${ZLIB_VERSION:-1.3.1}"
CURL_VERSION="${CURL_VERSION:-8.11.1}"
LIBPNG_VERSION="${LIBPNG_VERSION:-1.6.44}"
LIBJPEG_TURBO_VERSION="${LIBJPEG_TURBO_VERSION:-3.0.4}"
FREETYPE_VERSION="${FREETYPE_VERSION:-2.13.3}"
ICU_VERSION="${ICU_VERSION:-74.2}"
ANDROID_API="${ANDROID_API:-26}"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/php-out}"
WORK_DIR="${WORK_DIR:-$PWD/php-work/$SUFFIX}"

# Daftar ekstensi wajib (dipakai verifikasi simbol; urutan bebas).
EXTENSIONS=(
  bcmath ctype dom fileinfo filter hash json libxml mbstring openssl
  pdo pdo_sqlite phar posix session simplexml sockets sqlite3 tokenizer
  xml xmlreader xmlwriter zip zlib curl gd exif soap ftp pcntl intl
)

echo "== Build PHP ${PHP_VERSION} CLI+FPM (android/arm64, API ${ANDROID_API}) =="
echo "Ekstensi wajib: ${EXTENSIONS[*]}"

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
if [ -x "$TOOLCHAIN/bin/llvm-readelf" ]; then
  READELF="$TOOLCHAIN/bin/llvm-readelf"
else
  READELF="readelf"
fi
if [ -x "$TOOLCHAIN/bin/llvm-nm" ]; then
  NM="$TOOLCHAIN/bin/llvm-nm"
else
  NM="nm"
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
mkdir -p "$OUTPUT_DIR" "$PREFIX/lib/pkgconfig" "$PREFIX/include" "$SRC" "$DL"

download() {
  local url="$1" out="$2"
  if [ ! -f "$out" ]; then
    echo "-- Unduh $url --"
    curl -fsSL --retry 3 -o "$out.tmp" "$url"
    mv "$out.tmp" "$out"
  fi
}

CMAKE_ANDROID_ARGS=(
  -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake"
  -DANDROID_ABI=arm64-v8a
  -DANDROID_PLATFORM="android-${ANDROID_API}"
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_INSTALL_PREFIX="$PREFIX"
  -DCMAKE_PREFIX_PATH="$PREFIX"
  -DCMAKE_FIND_ROOT_PATH="$PREFIX"
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON
)

# --- zlib (statis) -----------------------------------------------------------
# PHP configure mencari zlib lewat pkg-config; sysroot NDK tidak
# menyediakan zlib.pc, jadi bangun zlib statis sendiri.
echo "-- Build zlib ${ZLIB_VERSION} (statis) --"
download "https://github.com/madler/zlib/releases/download/v${ZLIB_VERSION}/zlib-${ZLIB_VERSION}.tar.gz" "$DL/zlib.tar.gz"
rm -rf "$SRC/zlib-${ZLIB_VERSION}"
tar -xzf "$DL/zlib.tar.gz" -C "$SRC"
(
  cd "$SRC/zlib-${ZLIB_VERSION}"
  CC="$CC" AR="$AR" RANLIB="$RANLIB" ./configure --prefix="$PREFIX" --static
  make -j"$JOBS"
  make install
)

# --- SQLite (amalgamation, statis) --------------------------------------------
echo "-- Build SQLite ${SQLITE_VERSION} (amalgamation, statis) --"
download "https://www.sqlite.org/2024/sqlite-amalgamation-${SQLITE_AMALGAMATION}.zip" "$DL/sqlite.zip"
rm -rf "$SRC/sqlite-amalgamation-${SQLITE_AMALGAMATION}"
unzip -q "$DL/sqlite.zip" -d "$SRC"
(
  cd "$SRC/sqlite-amalgamation-${SQLITE_AMALGAMATION}"
  "$CC" -O2 -fPIC -DSQLITE_THREADSAFE=1 -DSQLITE_ENABLE_COLUMN_METADATA \
    -DSQLITE_ENABLE_FTS4 -DSQLITE_ENABLE_FTS5 -DSQLITE_ENABLE_RTREE \
    -c sqlite3.c -o sqlite3.o
  "$AR" rcs "$PREFIX/lib/libsqlite3.a" sqlite3.o
  cp sqlite3.h sqlite3ext.h "$PREFIX/include/"
)
cat > "$PREFIX/lib/pkgconfig/sqlite3.pc" <<EOF
prefix=$PREFIX
exec_prefix=\${prefix}
libdir=\${exec_prefix}/lib
includedir=\${prefix}/include

Name: SQLite
Description: SQL database engine
Version: $SQLITE_VERSION
Libs: -L\${libdir} -lsqlite3
Libs.private: -lm -ldl
Cflags: -I\${includedir}
EOF

# --- libxml2 (statis, tanpa iconv) ---------------------------------------------
echo "-- Build libxml2 ${LIBXML2_VERSION} (statis) --"
LIBXML2_SERIES=$(echo "$LIBXML2_VERSION" | cut -d. -f1,2)
download "https://download.gnome.org/sources/libxml2/${LIBXML2_SERIES}/libxml2-${LIBXML2_VERSION}.tar.xz" "$DL/libxml2.tar.xz"
rm -rf "$SRC/libxml2-${LIBXML2_VERSION}" "$SRC/libxml2-build"
tar -xf "$DL/libxml2.tar.xz" -C "$SRC"
cmake -S "$SRC/libxml2-${LIBXML2_VERSION}" -B "$SRC/libxml2-build" \
  "${CMAKE_ANDROID_ARGS[@]}" \
  -DBUILD_SHARED_LIBS=OFF \
  -DLIBXML2_WITH_ICONV=OFF \
  -DLIBXML2_WITH_PYTHON=OFF \
  -DLIBXML2_WITH_LZMA=OFF \
  -DLIBXML2_WITH_ZLIB=ON \
  -DLIBXML2_WITH_THREADS=ON \
  -DLIBXML2_WITH_HTTP=OFF \
  -DLIBXML2_WITH_FTP=OFF \
  -DLIBXML2_WITH_TESTS=OFF
cmake --build "$SRC/libxml2-build" -j"$JOBS"
cmake --install "$SRC/libxml2-build"
if [ ! -f "$PREFIX/lib/pkgconfig/libxml-2.0.pc" ]; then
  echo "libxml-2.0.pc tidak terinstall; menulis pkgconfig manual"
  cat > "$PREFIX/lib/pkgconfig/libxml-2.0.pc" <<EOF
prefix=$PREFIX
exec_prefix=\${prefix}
libdir=\${exec_prefix}/lib
includedir=\${prefix}/include

Name: libXML
Description: libXML library version2.
Version: $LIBXML2_VERSION
Libs: -L\${libdir} -lxml2
Libs.private: -lz -lm -ldl
Cflags: -I\${includedir}/libxml2
EOF
fi

# --- OpenSSL (statis) ------------------------------------------------------------
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

# --- Oniguruma (statis) ------------------------------------------------------------
echo "-- Build Oniguruma ${ONIGURUMA_VERSION} (statis) --"
download "https://github.com/kkos/oniguruma/releases/download/v${ONIGURUMA_VERSION}/onig-${ONIGURUMA_VERSION}.tar.gz" "$DL/onig.tar.gz"
rm -rf "$SRC/onig-${ONIGURUMA_VERSION}"
tar -xzf "$DL/onig.tar.gz" -C "$SRC"
(
  cd "$SRC/onig-${ONIGURUMA_VERSION}"
  ./configure --host=aarch64-linux-android --prefix="$PREFIX" \
    --enable-static --disable-shared CC="$CC" AR="$AR" RANLIB="$RANLIB"
  make -j"$JOBS"
  make install
)

# --- libzip (statis) -----------------------------------------------------------------
echo "-- Build libzip ${LIBZIP_VERSION} (statis) --"
download "https://libzip.org/download/libzip-${LIBZIP_VERSION}.tar.gz" "$DL/libzip.tar.gz"
rm -rf "$SRC/libzip-${LIBZIP_VERSION}" "$SRC/libzip-build"
tar -xzf "$DL/libzip.tar.gz" -C "$SRC"
cmake -S "$SRC/libzip-${LIBZIP_VERSION}" -B "$SRC/libzip-build" \
  "${CMAKE_ANDROID_ARGS[@]}" \
  -DBUILD_SHARED_LIBS=OFF \
  -DENABLE_COMMONCRYPTO=OFF \
  -DENABLE_GNUTLS=OFF \
  -DENABLE_OPENSSL=OFF \
  -DENABLE_MBEDTLS=OFF \
  -DENABLE_BZIP2=OFF \
  -DENABLE_LZMA=OFF \
  -DENABLE_ZSTD=OFF \
  -DBUILD_TOOLS=OFF \
  -DBUILD_REGRESS=OFF \
  -DBUILD_EXAMPLES=OFF \
  -DBUILD_DOC=OFF
cmake --build "$SRC/libzip-build" -j"$JOBS"
cmake --install "$SRC/libzip-build"

# --- libpng (statis) -------------------------------------------------------------------
echo "-- Build libpng ${LIBPNG_VERSION} (statis) --"
download "https://download.sourceforge.net/libpng/libpng-${LIBPNG_VERSION}.tar.gz" "$DL/libpng.tar.gz"
rm -rf "$SRC/libpng-${LIBPNG_VERSION}" "$SRC/libpng-build"
tar -xzf "$DL/libpng.tar.gz" -C "$SRC"
cmake -S "$SRC/libpng-${LIBPNG_VERSION}" -B "$SRC/libpng-build" \
  "${CMAKE_ANDROID_ARGS[@]}" \
  -DPNG_SHARED=OFF \
  -DPNG_STATIC=ON \
  -DPNG_TESTS=OFF \
  -DPNG_TOOLS=OFF
cmake --build "$SRC/libpng-build" -j"$JOBS"
cmake --install "$SRC/libpng-build"
# ext/gd mencari pkg-config "libpng"; CMake libpng memasang libpng16.pc.
if [ ! -f "$PREFIX/lib/pkgconfig/libpng.pc" ] && [ -f "$PREFIX/lib/pkgconfig/libpng16.pc" ]; then
  cp "$PREFIX/lib/pkgconfig/libpng16.pc" "$PREFIX/lib/pkgconfig/libpng.pc"
fi

# --- libjpeg-turbo (statis) ---------------------------------------------------------------
echo "-- Build libjpeg-turbo ${LIBJPEG_TURBO_VERSION} (statis) --"
download "https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/${LIBJPEG_TURBO_VERSION}/libjpeg-turbo-${LIBJPEG_TURBO_VERSION}.tar.gz" "$DL/libjpeg-turbo.tar.gz"
rm -rf "$SRC/libjpeg-turbo-${LIBJPEG_TURBO_VERSION}" "$SRC/libjpeg-build"
tar -xzf "$DL/libjpeg-turbo.tar.gz" -C "$SRC"
cmake -S "$SRC/libjpeg-turbo-${LIBJPEG_TURBO_VERSION}" -B "$SRC/libjpeg-build" \
  "${CMAKE_ANDROID_ARGS[@]}" \
  -DENABLE_SHARED=OFF \
  -DENABLE_STATIC=ON \
  -DWITH_TURBOJPEG=OFF
cmake --build "$SRC/libjpeg-build" -j"$JOBS"
cmake --install "$SRC/libjpeg-build"
if [ ! -f "$PREFIX/lib/pkgconfig/libjpeg.pc" ]; then
  echo "libjpeg.pc tidak terinstall; menulis pkgconfig manual"
  cat > "$PREFIX/lib/pkgconfig/libjpeg.pc" <<EOF
prefix=$PREFIX
exec_prefix=\${prefix}
libdir=\${exec_prefix}/lib
includedir=\${prefix}/include

Name: libjpeg
Description: libjpeg-turbo JPEG library
Version: $LIBJPEG_TURBO_VERSION
Libs: -L\${libdir} -ljpeg
Cflags: -I\${includedir}
EOF
fi

# --- freetype (statis) -----------------------------------------------------------------------
echo "-- Build freetype ${FREETYPE_VERSION} (statis) --"
download "https://download.savannah.gnu.org/releases/freetype/freetype-${FREETYPE_VERSION}.tar.gz" "$DL/freetype.tar.gz"
rm -rf "$SRC/freetype-${FREETYPE_VERSION}" "$SRC/freetype-build"
tar -xzf "$DL/freetype.tar.gz" -C "$SRC"
cmake -S "$SRC/freetype-${FREETYPE_VERSION}" -B "$SRC/freetype-build" \
  "${CMAKE_ANDROID_ARGS[@]}" \
  -DBUILD_SHARED_LIBS=OFF \
  -DFT_DISABLE_HARFBUZZ=ON \
  -DFT_DISABLE_BROTLI=ON \
  -DFT_DISABLE_BZIP2=ON \
  -DFT_DISABLE_PNG=ON
cmake --build "$SRC/freetype-build" -j"$JOBS"
cmake --install "$SRC/freetype-build"

# --- libcurl (statis, OpenSSL + zlib) ------------------------------------------------------------
echo "-- Build libcurl ${CURL_VERSION} (statis) --"
download "https://curl.se/download/curl-${CURL_VERSION}.tar.gz" "$DL/curl.tar.gz"
rm -rf "$SRC/curl-${CURL_VERSION}" "$SRC/curl-build"
tar -xzf "$DL/curl.tar.gz" -C "$SRC"
cmake -S "$SRC/curl-${CURL_VERSION}" -B "$SRC/curl-build" \
  "${CMAKE_ANDROID_ARGS[@]}" \
  -DBUILD_SHARED_LIBS=OFF \
  -DBUILD_STATIC_LIBS=ON \
  -DBUILD_CURL_EXE=OFF \
  -DBUILD_TESTING=OFF \
  -DCURL_ENABLE_SSL=ON \
  -DCURL_USE_OPENSSL=ON \
  -DOPENSSL_ROOT_DIR="$PREFIX" \
  -DOPENSSL_INCLUDE_DIR="$PREFIX/include" \
  -DOPENSSL_CRYPTO_LIBRARY="$PREFIX/lib/libcrypto.a" \
  -DOPENSSL_SSL_LIBRARY="$PREFIX/lib/libssl.a" \
  -DOPENSSL_USE_STATIC_LIBS=ON \
  -DCURL_ZLIB=ON \
  -DCURL_CA_BUNDLE=none \
  -DCURL_CA_PATH=none \
  -DCURL_DISABLE_LDAP=ON \
  -DCURL_DISABLE_LDAPS=ON \
  -DCURL_USE_LIBPSL=OFF \
  -DCURL_USE_LIBSSH2=OFF \
  -DUSE_LIBIDN2=OFF \
  -DCURL_BROTLI=OFF \
  -DCURL_ZSTD=OFF \
  -DENABLE_ARES=OFF \
  -DUSE_NGHTTP2=OFF
cmake --build "$SRC/curl-build" -j"$JOBS"
cmake --install "$SRC/curl-build"

# --- ICU (statis, data packaging archive) -----------------------------------------------------------
# ICU butuh cross-build: tools ICU (genrb, pkgdata, dst.) harus berjalan
# di host, jadi ICU dibangun dulu untuk host lalu target Android memakai
# --with-cross-build. Data dikemas sebagai archive icudt74l.dat yang
# disalin core ke data/assets dan ditunjuk lewat ICU_DATA saat runtime.
echo "-- Build ICU ${ICU_VERSION} host (cross-build tools) --"
ICU_TARBALL="icu4c-$(echo "$ICU_VERSION" | tr '.' '_')-src.tgz"
download "https://github.com/unicode-org/icu/releases/download/release-${ICU_VERSION//./-}/${ICU_TARBALL}" "$DL/icu.tgz"
rm -rf "$SRC/icu" "$SRC/icu-host-build" "$SRC/icu-target-build"
tar -xzf "$DL/icu.tgz" -C "$SRC"
ICU_SRC="$SRC/icu/source"
ICU_HOST_BUILD="$SRC/icu-host-build"
mkdir -p "$ICU_HOST_BUILD"
(
  cd "$ICU_HOST_BUILD"
  "$ICU_SRC/runConfigureICU" Linux/gcc --disable-tests --disable-samples
  make -j"$JOBS"
)
echo "-- Build ICU ${ICU_VERSION} target Android (statis, data archive) --"
ICU_TARGET_BUILD="$SRC/icu-target-build"
mkdir -p "$ICU_TARGET_BUILD"
(
  cd "$ICU_TARGET_BUILD"
  CC="$CC" CXX="$CXX" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP" \
  CFLAGS="-O2 -fPIC" CXXFLAGS="-O2 -fPIC" \
  "$ICU_SRC/configure" \
    --host=aarch64-linux-android \
    --with-cross-build="$ICU_HOST_BUILD" \
    --prefix="$PREFIX" \
    --enable-static \
    --disable-shared \
    --disable-tests \
    --disable-samples \
    --disable-extras \
    --disable-layout \
    --disable-layoutex \
    --with-data-packaging=archive
  make -j"$JOBS"
  make install
)
ICUDT_SRC=""
for candidate in \
  "$PREFIX/share/icu/${ICU_VERSION}/icudt74l.dat" \
  "$ICU_TARGET_BUILD/data/out/icudt74l.dat"; do
  if [ -f "$candidate" ]; then
    ICUDT_SRC="$candidate"
    break
  fi
done
if [ -z "$ICUDT_SRC" ]; then
  ICUDT_SRC=$(find "$PREFIX" "$ICU_TARGET_BUILD" -name "icudt74l.dat" -type f 2>/dev/null | head -1 || true)
fi
if [ -z "$ICUDT_SRC" ] || [ ! -f "$ICUDT_SRC" ]; then
  echo "::error::icudt74l.dat tidak ditemukan setelah build ICU ${ICU_VERSION} (data packaging archive gagal)" >&2
  exit 1
fi
cp "$ICUDT_SRC" "$OUTPUT_DIR/icudt74l.dat"
ICUDT_SIZE=$(stat -c%s "$OUTPUT_DIR/icudt74l.dat" 2>/dev/null || wc -c < "$OUTPUT_DIR/icudt74l.dat")
if [ "$ICUDT_SIZE" -le 1000000 ]; then
  echo "::error::icudt74l.dat terlalu kecil ($ICUDT_SIZE bytes), diduga data ICU tidak lengkap" >&2
  exit 1
fi
echo "icudt74l.dat: $OUTPUT_DIR/icudt74l.dat ($ICUDT_SIZE bytes)"

# --- PHP CLI + FPM -------------------------------------------------------------------------------------
echo "-- Build PHP ${PHP_VERSION} (CLI + FPM) --"
download "https://www.php.net/distributions/php-${PHP_VERSION}.tar.gz" "$DL/php.tar.gz"
rm -rf "$SRC/php-${PHP_VERSION}"
tar -xzf "$DL/php.tar.gz" -C "$SRC"

# libc++ statis untuk intl/ICU (C++): ambil arsip yang benar-benar ada
# di sysroot NDK agar DT_NEEDED tetap hanya Bionic.
SYSROOT_LIB="$TOOLCHAIN/sysroot/usr/lib/aarch64-linux-android"
CXX_STATIC_LIBS=""
for lib in c++_static c++abi c++demangle; do
  if [ -f "$SYSROOT_LIB/lib${lib}.a" ]; then
    CXX_STATIC_LIBS="$CXX_STATIC_LIBS -l${lib}"
  fi
done
if [ -z "$CXX_STATIC_LIBS" ]; then
  echo "::error::libc++ statis tidak ditemukan di $SYSROOT_LIB (libc++_static.a); intl tidak bisa ditautkan tanpa libc++_shared.so" >&2
  exit 1
fi
echo "libc++ statis untuk intl:$CXX_STATIC_LIBS"

(
  cd "$SRC/php-${PHP_VERSION}"
  export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
  export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
  export PKG_CONFIG="pkg-config --static"
  export CFLAGS="-O2 -fPIC"
  export CXXFLAGS="-O2 -fPIC"
  export CPPFLAGS="-I$PREFIX/include"
  export LDFLAGS="-L$PREFIX/lib"
  ./configure \
    --host=aarch64-linux-android \
    --build=x86_64-linux-gnu \
    --prefix="$PREFIX" \
    --with-config-file-path="$PREFIX/etc" \
    --disable-all \
    --disable-cgi \
    --disable-phpdbg \
    --without-pear \
    --enable-fpm \
    --enable-bcmath \
    --enable-ctype \
    --with-libxml \
    --enable-dom \
    --enable-fileinfo \
    --enable-filter \
    --enable-mbstring \
    --with-openssl \
    --enable-pdo \
    --with-pdo-sqlite \
    --enable-phar \
    --enable-posix \
    --enable-session \
    --enable-simplexml \
    --enable-sockets \
    --with-sqlite3 \
    --enable-tokenizer \
    --enable-xml \
    --enable-xmlreader \
    --enable-xmlwriter \
    --with-zip \
    --with-zlib \
    --with-curl \
    --enable-gd \
    --with-jpeg \
    --with-freetype \
    --enable-exif \
    --enable-soap \
    --enable-ftp \
    --enable-pcntl \
    --enable-intl \
    CC="$CC" CXX="$CXX" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP" \
    LIBS="$CXX_STATIC_LIBS"
  # Patch Bionic (lihat catatan header): matikan deteksi res_n*/dn_*
  # dan getdtablesize di php_config.h.
  for sym in HAVE_RES_NINIT HAVE_RES_NSEARCH HAVE_RES_NCLOSE HAVE_RES_SEARCH HAVE_RES_INIT HAVE_DN_SKIPNAME HAVE_DN_EXPAND HAVE_GETDTABLESIZE; do
    sed -i "s/^#define $sym 1$/\/* #undef $sym *\//" main/php_config.h
  done
  grep -E "HAVE_RES_|HAVE_DN_" main/php_config.h || true
  # Bionic tidak mendeklarasikan getdtablesize(); ganti sysconf setara.
  grep -rl "getdtablesize()" ext main sapi | xargs sed -i 's/getdtablesize()/sysconf(_SC_OPEN_MAX)/g'
  make -j"$JOBS"
)

PHP_CLI_BIN="$SRC/php-${PHP_VERSION}/sapi/cli/php"
PHP_FPM_BIN="$SRC/php-${PHP_VERSION}/sapi/fpm/php-fpm"
for bin in "$PHP_CLI_BIN" "$PHP_FPM_BIN"; do
  if [ ! -f "$bin" ]; then
    echo "::error::Binary PHP hasil build tidak ditemukan: $bin" >&2
    exit 1
  fi
done

# --- Verifikasi simbol ekstensi (sebelum strip) -----------------------------------------------------------
# Strip menghapus symtab, jadi pemeriksaan phpext_<ext>_ptr wajib pada
# binary hasil build yang belum di-strip.
verify_extension_symbols() {
  local bin="$1" label="$2"
  local nm_out missing=()
  nm_out=$("$NM" "$bin" 2>/dev/null || true)
  if [ -z "$nm_out" ]; then
    echo "::error::Tidak bisa membaca simbol $label ($bin) dengan $NM" >&2
    exit 1
  fi
  for ext in "${EXTENSIONS[@]}"; do
    # Ekstensi statis mengekspor <ext>_module_entry; phpext_<ext>_ptr
    # hanya ada pada build shared. Terima salah satu sebagai bukti.
    if ! printf '%s\n' "$nm_out" | grep -E "[[:space:]](phpext_${ext}_ptr|${ext}_module_entry)$" >/dev/null; then
      missing+=("$ext")
    fi
  done
  if [ "${#missing[@]}" -ne 0 ]; then
    echo "::error::Ekstensi hilang pada $label: ${missing[*]} (simbol <ext>_module_entry/phpext_<ext>_ptr tidak ditemukan)" >&2
    exit 1
  fi
  echo "OK: ${#EXTENSIONS[@]} ekstensi terverifikasi pada $label"
}
verify_extension_symbols "$PHP_CLI_BIN" "PHP ${PHP_VERSION} CLI"
verify_extension_symbols "$PHP_FPM_BIN" "PHP ${PHP_VERSION} FPM"

# --- Paket + verifikasi ELF/DT_NEEDED ------------------------------------------------------------------------
package_binary() {
  local src="$1" out="$2"
  cp "$src" "$out"
  "$STRIP" "$out"
  chmod +x "$out"
  if command -v file >/dev/null 2>&1; then
    file "$out"
  fi
  "$READELF" -h "$out" | grep -E "Class|Type|Machine"
  if ! "$READELF" -h "$out" | grep "AArch64" >/dev/null; then
    echo "::error::$out bukan ELF AArch64" >&2
    exit 1
  fi
  "$READELF" -d "$out" | grep NEEDED || echo "(tidak ada DT_NEEDED)"
  for lib in $("$READELF" -d "$out" | grep NEEDED | sed -E 's/.*\[(.*)\]/\1/' || true); do
    case "$lib" in
      libc.so|libm.so|libdl.so|libz.so|liblog.so) ;;
      *)
        echo "::error::Dependensi dinamis tak terduga pada $out: $lib (dependensi harus statis)" >&2
        exit 1
        ;;
    esac
  done
  local size
  size=$(stat -c%s "$out" 2>/dev/null || wc -c < "$out")
  echo "Selesai: $out ($size bytes)"
}
package_binary "$PHP_CLI_BIN" "$OUTPUT_DIR/libphp${SUFFIX}.so"
package_binary "$PHP_FPM_BIN" "$OUTPUT_DIR/libphpfpm${SUFFIX}.so"

ls -lh "$OUTPUT_DIR/libphp${SUFFIX}.so" "$OUTPUT_DIR/libphpfpm${SUFFIX}.so" "$OUTPUT_DIR/icudt74l.dat"
echo "== Build PHP ${PHP_VERSION} selesai =="
