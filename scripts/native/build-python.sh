#!/usr/bin/env bash
set -euo pipefail

# WebForge Mobile — cross-compile CPython untuk Android arm64.
#
# Status worker: script ini BELUM teruji lokal di sandbox (tidak ada NDK
# dan perangkat Android di lingkungan worker). Verifikasi pertama harus
# dilakukan di CI, lalu di POCO M6. Script gagal keras bila modul kritis
# tidak menjadi statis atau DT_NEEDED mengandung library non-bionic.
#
# Input environment:
#   ANDROID_NDK_HOME  wajib, path Android NDK
#   ANDROID_API       default 26
#   OUTPUT_DIR        default ./python-out
#   WORK_DIR          default ./python-build
#
# Output:
#   $OUTPUT_DIR/libpython.so
#   $OUTPUT_DIR/python-stdlib.zip
#   $OUTPUT_DIR/python-version.txt

PYTHON_VERSION="3.12.7"
PYTHON_SERIES="3.12"
OPENSSL_VERSION="3.3.2"
SQLITE_VERSION="3.46.1"
SQLITE_AMALGAMATION="3460100"
ZLIB_VERSION="1.3.1"
LIBFFI_VERSION="3.4.6"
BZIP2_VERSION="1.0.8"
XZ_VERSION="5.4.6"

ANDROID_API="${ANDROID_API:-26}"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/python-out}"
WORK_DIR="${WORK_DIR:-$PWD/python-build}"
: "${ANDROID_NDK_HOME:?ANDROID_NDK_HOME wajib diisi}"

mkdir -p "$OUTPUT_DIR" "$WORK_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
WORK_DIR="$(cd "$WORK_DIR" && pwd)"
PREFIX="$WORK_DIR/prefix"
SRC_DIR="$WORK_DIR/src"
HOST_PREFIX="$WORK_DIR/host-python-prefix"
INSTALL_ROOT="$WORK_DIR/install-root"
mkdir -p "$PREFIX/include" "$PREFIX/lib/pkgconfig" "$SRC_DIR"

TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
if [ ! -d "$TOOLCHAIN" ]; then
  echo "Toolchain NDK linux-x86_64 tidak ditemukan: $TOOLCHAIN" >&2
  exit 1
fi
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
export PATH="$TOOLCHAIN/bin:$PATH"
export CC="$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang"
export CXX="$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang++"
export AR="$TOOLCHAIN/bin/llvm-ar"
export RANLIB="$TOOLCHAIN/bin/llvm-ranlib"
export STRIP="$TOOLCHAIN/bin/llvm-strip"
READELF="$TOOLCHAIN/bin/llvm-readelf"
[ -x "$READELF" ] || READELF="readelf"
for tool in "$CC" "$CXX" "$AR" "$RANLIB" "$STRIP"; do
  if [ ! -x "$tool" ]; then
    echo "Tool NDK tidak ditemukan: $tool" >&2
    exit 1
  fi
done
JOBS="$(nproc 2>/dev/null || echo 4)"

log() {
  printf '\n==> %s\n' "$*"
}

download() {
  local url="$1"
  local dest="$2"
  if [ ! -f "$dest" ]; then
    curl -fsSL -o "$dest.tmp" "$url"
    mv "$dest.tmp" "$dest"
  fi
}

extract_to() {
  local archive="$1"
  local dest="$2"
  if [ ! -f "$dest/configure" ] && [ ! -f "$dest/Makefile" ] && [ ! -f "$dest/sqlite3.c" ]; then
    rm -rf "$dest"
    mkdir -p "$dest"
    tar xf "$archive" -C "$dest" --strip-components=1
  fi
}

find_build_python() {
  if command -v python3.12 >/dev/null 2>&1; then
    command -v python3.12
    return
  fi
  if command -v python3 >/dev/null 2>&1 && python3 -c 'import sys; raise SystemExit(0 if sys.version_info[:2] == (3, 12) else 1)'; then
    command -v python3
    return
  fi
  echo ""
}

build_host_python_if_needed() {
  BUILD_PYTHON="$(find_build_python)"
  if [ -n "$BUILD_PYTHON" ]; then
    log "Memakai build Python host: $BUILD_PYTHON"
    return
  fi
  log "Build Python host $PYTHON_VERSION untuk cross-compile"
  local archive="$SRC_DIR/Python-${PYTHON_VERSION}.tar.xz"
  download "https://www.python.org/ftp/python/${PYTHON_VERSION}/Python-${PYTHON_VERSION}.tar.xz" "$archive"
  local host_src="$SRC_DIR/host-python-src"
  extract_to "$archive" "$host_src"
  (
    cd "$host_src"
    env CC=cc CXX=c++ AR=ar RANLIB=ranlib CFLAGS= CPPFLAGS= LDFLAGS= \
      ./configure --prefix="$HOST_PREFIX" --disable-test-modules
    env CC=cc CXX=c++ AR=ar RANLIB=ranlib make -j"$JOBS"
    env CC=cc CXX=c++ AR=ar RANLIB=ranlib make install
  )
  BUILD_PYTHON="$HOST_PREFIX/bin/python3.12"
  if [ ! -x "$BUILD_PYTHON" ]; then
    echo "Build Python host gagal: $BUILD_PYTHON tidak ada" >&2
    exit 1
  fi
}

build_zlib() {
  [ -f "$PREFIX/lib/libz.a" ] && return
  log "Build zlib $ZLIB_VERSION (statis)"
  local archive="$SRC_DIR/zlib-${ZLIB_VERSION}.tar.gz"
  download "https://github.com/madler/zlib/releases/download/v${ZLIB_VERSION}/zlib-${ZLIB_VERSION}.tar.gz" "$archive"
  local src="$SRC_DIR/zlib-src"
  extract_to "$archive" "$src"
  (
    cd "$src"
    CC="$CC" AR="$AR" RANLIB="$RANLIB" ./configure --prefix="$PREFIX" --static
    make -j"$JOBS"
    make install
  )
}

build_bzip2() {
  [ -f "$PREFIX/lib/libbz2.a" ] && return
  log "Build bzip2 $BZIP2_VERSION (statis)"
  local archive="$SRC_DIR/bzip2-${BZIP2_VERSION}.tar.gz"
  download "https://www.sourceware.org/pub/bzip2/bzip2-${BZIP2_VERSION}.tar.gz" "$archive"
  local src="$SRC_DIR/bzip2-src"
  extract_to "$archive" "$src"
  (
    cd "$src"
    make CC="$CC" AR="$AR" RANLIB="$RANLIB" CFLAGS="-O2 -fPIC -D_FILE_OFFSET_BITS=64" libbz2.a
    cp libbz2.a "$PREFIX/lib/"
    cp bzlib.h "$PREFIX/include/"
  )
}

build_xz() {
  [ -f "$PREFIX/lib/liblzma.a" ] && return
  log "Build xz $XZ_VERSION (statis)"
  local archive="$SRC_DIR/xz-${XZ_VERSION}.tar.gz"
  download "https://github.com/tukaani-project/xz/releases/download/v${XZ_VERSION}/xz-${XZ_VERSION}.tar.gz" "$archive"
  local src="$SRC_DIR/xz-src"
  extract_to "$archive" "$src"
  (
    cd "$src"
    ./configure --host=aarch64-linux-android --prefix="$PREFIX" \
      --enable-static --disable-shared --disable-nls --disable-doc \
      --disable-scripts CC="$CC" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP"
    make -j"$JOBS"
    make install
  )
}

build_libffi() {
  [ -f "$PREFIX/lib/libffi.a" ] && return
  log "Build libffi $LIBFFI_VERSION (statis)"
  local archive="$SRC_DIR/libffi-${LIBFFI_VERSION}.tar.gz"
  download "https://github.com/libffi/libffi/releases/download/v${LIBFFI_VERSION}/libffi-${LIBFFI_VERSION}.tar.gz" "$archive"
  local src="$SRC_DIR/libffi-src"
  extract_to "$archive" "$src"
  (
    cd "$src"
    ./configure --host=aarch64-linux-android --prefix="$PREFIX" \
      --enable-static --disable-shared --disable-docs --with-pic \
      CC="$CC" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP"
    make -j"$JOBS"
    make install
  )
}

build_openssl() {
  [ -f "$PREFIX/lib/libssl.a" ] && [ -f "$PREFIX/lib/libcrypto.a" ] && return
  log "Build OpenSSL $OPENSSL_VERSION (statis)"
  local archive="$SRC_DIR/openssl-${OPENSSL_VERSION}.tar.gz"
  download "https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VERSION}/openssl-${OPENSSL_VERSION}.tar.gz" "$archive"
  local src="$SRC_DIR/openssl-src"
  extract_to "$archive" "$src"
  (
    cd "$src"
    ./Configure android-arm64 "-D__ANDROID_API__=${ANDROID_API}" \
      no-shared no-tests no-docs \
      --prefix="$PREFIX" --openssldir="$PREFIX/ssl"
    make -j"$JOBS" build_sw
    make install_sw
  )
}

build_sqlite() {
  [ -f "$PREFIX/lib/libsqlite3.a" ] && return
  log "Build SQLite $SQLITE_VERSION (statis)"
  local archive="$SRC_DIR/sqlite-amalgamation-${SQLITE_AMALGAMATION}.zip"
  download "https://www.sqlite.org/2024/sqlite-amalgamation-${SQLITE_AMALGAMATION}.zip" "$archive"
  local src="$SRC_DIR/sqlite-amalgamation-${SQLITE_AMALGAMATION}"
  if [ ! -f "$src/sqlite3.c" ]; then
    rm -rf "$src"
    unzip -q -o "$archive" -d "$SRC_DIR"
  fi
  (
    cd "$src"
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
}

force_static_modules() {
  log "Memaksa modul ekstensi esensial menjadi statis"
  "$BUILD_PYTHON" - <<'PY'
import pathlib
import sys

setup_files = [pathlib.Path("Modules/Setup.stdlib"), pathlib.Path("Modules/Setup")]
setup_files = [f for f in setup_files if f.is_file()]
local = pathlib.Path("Modules/Setup.local")
targets = {
    "_ssl", "_hashlib", "_sqlite3", "_ctypes", "_socket", "_json",
    "_asyncio", "_multiprocessing", "_queue", "_datetime", "_decimal",
    "zlib", "_bz2", "_lzma", "_struct", "_random", "_bisect", "_heapq",
}
critical = {"_ssl", "_hashlib", "_sqlite3", "_socket", "zlib"}
if not setup_files:
    print("Modules/Setup.stdlib/Setup tidak ditemukan setelah configure", file=sys.stderr)
    raise SystemExit(1)

moved = []
found = set()
for setup in setup_files:
    kept = []
    for line in setup.read_text().splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or stripped.startswith("*"):
            kept.append(line)
            continue
        first = stripped.split()[0]
        if first in targets:
            moved.append(line)
            found.add(first)
        else:
            kept.append(line)
    setup.write_text("\n".join(kept) + "\n")

missing_critical = sorted(critical - found)
if missing_critical:
    print(f"Modul kritis tidak ada di Setup.stdlib/Setup: {missing_critical}", file=sys.stderr)
    raise SystemExit(1)
missing = sorted(targets - found)
if missing:
    print(f"Peringatan: modul nonkritis tidak ditemukan untuk dijadikan statis: {missing}", file=sys.stderr)

local.write_text("*static*\n" + "\n".join(moved) + "\n")
print("Modul statis:", ", ".join(sorted(found)))
PY
}

verify_static_modules() {
  local config_c="Modules/config.c"
  if [ ! -f "$config_c" ]; then
    echo "Modules/config.c tidak ditemukan; tidak bisa memverifikasi modul statis" >&2
    exit 1
  fi
  for mod in _ssl _hashlib _sqlite3 _socket zlib; do
    if ! grep -q "PyInit_${mod}" "$config_c"; then
      echo "Modul kritis $mod tidak statis di libpython" >&2
      exit 1
    fi
  done
  for mod in _ctypes _json _asyncio _multiprocessing _queue _datetime _decimal _bz2 _lzma _struct _random _bisect _heapq; do
    if ! grep -q "PyInit_${mod}" "$config_c"; then
      echo "Peringatan: modul $mod tidak statis; fallback Python murni atau fitur terkait mungkin tidak tersedia" >&2
    fi
  done
}

verify_elf() {
  local file="$1"
  if [ ! -f "$file" ]; then
    echo "File ELF tidak ditemukan: $file" >&2
    exit 1
  fi
  file "$file" || true
  if ! "$READELF" -h "$file" | grep "AArch64" >/dev/null; then
    echo "$file bukan ELF AArch64" >&2
    exit 1
  fi
  "$READELF" -d "$file" | grep NEEDED || true
  for lib in $("$READELF" -d "$file" | grep NEEDED | sed -E 's/.*\[(.*)\]/\1/'); do
    case "$lib" in
      libc.so|libm.so|libdl.so|libz.so|liblog.so|libandroid.so) ;;
      *)
        echo "Dependensi dinamis non-bionic pada $file: $lib" >&2
        exit 1
        ;;
    esac
  done
}

build_python() {
  log "Build CPython $PYTHON_VERSION untuk aarch64-linux-android API $ANDROID_API"
  local archive="$SRC_DIR/Python-${PYTHON_VERSION}.tar.xz"
  download "https://www.python.org/ftp/python/${PYTHON_VERSION}/Python-${PYTHON_VERSION}.tar.xz" "$archive"
  local src="$SRC_DIR/python-target-src"
  extract_to "$archive" "$src"

  export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
  export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
  export CFLAGS="-O2 -fPIC"
  export CPPFLAGS="-I$PREFIX/include"
  export LDFLAGS="-L$PREFIX/lib"

  (
    cd "$src"
    ./configure \
      --host=aarch64-linux-android \
      --build=x86_64-linux-gnu \
      --prefix="$PREFIX" \
      --with-build-python="$BUILD_PYTHON" \
      --disable-shared \
      --disable-test-modules \
      --with-openssl="$PREFIX" \
      --with-system-ffi \
      --enable-ipv6 \
      --with-ensurepip=no \
      CC="$CC" CXX="$CXX" AR="$AR" RANLIB="$RANLIB" STRIP="$STRIP" \
      ac_cv_file__dev_ptmx=no ac_cv_file__dev_ptc=no
    force_static_modules
    make -j"$JOBS"
    verify_static_modules
    rm -rf "$INSTALL_ROOT"
    make install DESTDIR="$INSTALL_ROOT"
  )

  local installed_stdlib="$INSTALL_ROOT$PREFIX/lib/python${PYTHON_SERIES}"
  if [ ! -d "$installed_stdlib" ]; then
    echo "Stdlib hasil install tidak ditemukan: $installed_stdlib" >&2
    exit 1
  fi

  if [ -f "$src/python" ]; then
    cp "$src/python" "$OUTPUT_DIR/libpython.so"
  elif [ -f "$INSTALL_ROOT$PREFIX/bin/python${PYTHON_SERIES}" ]; then
    cp "$INSTALL_ROOT$PREFIX/bin/python${PYTHON_SERIES}" "$OUTPUT_DIR/libpython.so"
  else
    echo "Binary Python hasil build tidak ditemukan" >&2
    exit 1
  fi
  "$STRIP" "$OUTPUT_DIR/libpython.so"
  verify_elf "$OUTPUT_DIR/libpython.so"

  log "Mengemas stdlib Python murni"
  local stage="$WORK_DIR/stdlib-stage"
  rm -rf "$stage"
  mkdir -p "$stage"
  cp -a "$installed_stdlib/." "$stage/"
  rm -rf \
    "$stage/test" \
    "$stage/idlelib" \
    "$stage/tkinter" \
    "$stage/turtledemo" \
    "$stage/lib-dynload" \
    "$stage/site-packages"
  rm -f "$stage/turtle.py"
  find "$stage" -type d -name __pycache__ -prune -exec rm -rf {} +
  find "$stage" -type d -name 'config-*' -prune -exec rm -rf {} +
  find "$stage" -type f -name '*.pyc' -delete

  if [ ! -f "$stage/ensurepip/__init__.py" ]; then
    echo "ensurepip tidak ada di stdlib hasil install" >&2
    exit 1
  fi
  if ! ls "$stage"/ensurepip/_bundled/pip-*.whl >/dev/null 2>&1; then
    echo "Wheel pip bundled ensurepip tidak ditemukan" >&2
    exit 1
  fi

  rm -f "$OUTPUT_DIR/python-stdlib.zip"
  "$BUILD_PYTHON" - "$stage" "$OUTPUT_DIR/python-stdlib.zip" <<'PY'
import pathlib
import sys
import zipfile

stage = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_STORED) as zf:
    for path in sorted(stage.rglob("*")):
        if path.is_file():
            zf.write(path, path.relative_to(stage).as_posix())
with zipfile.ZipFile(out) as zf:
    names = set(zf.namelist())
    for required in ("encodings/__init__.py", "ensurepip/__init__.py", "os.py"):
        if required not in names:
            raise SystemExit(f"Stdlib zip tidak berisi {required}")
PY

  printf '%s\n' "$PYTHON_VERSION" > "$OUTPUT_DIR/python-version.txt"
  log "Selesai: $OUTPUT_DIR/libpython.so, $OUTPUT_DIR/python-stdlib.zip, $OUTPUT_DIR/python-version.txt"
}

build_host_python_if_needed
build_zlib
build_bzip2
build_xz
build_libffi
build_openssl
build_sqlite
build_python
