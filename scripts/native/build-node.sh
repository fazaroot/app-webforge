#!/usr/bin/env bash
set -euo pipefail

# WebForge Mobile — menyiapkan Node.js dari nodejs-mobile dan shim JNI.
#
# Status worker: script ini BELUM teruji lokal di sandbox (tidak ada NDK
# dan perangkat Android di lingkungan worker). Verifikasi pertama harus
# dilakukan di CI, lalu di POCO M6.
#
# Input environment:
#   ANDROID_NDK_HOME  wajib, path Android NDK
#   ANDROID_API       default 26
#   OUTPUT_DIR        default ./node-out
#   WORK_DIR          default ./node-build
#
# Output:
#   $OUTPUT_DIR/libnode.so
#   $OUTPUT_DIR/libwebforge_node.so
#   $OUTPUT_DIR/npm.tgz
#   $OUTPUT_DIR/node-version.txt

NODEJS_MOBILE_VERSION="18.20.4"
NPM_VERSION="10.8.2"

ANDROID_API="${ANDROID_API:-26}"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/node-out}"
WORK_DIR="${WORK_DIR:-$PWD/node-build}"
: "${ANDROID_NDK_HOME:?ANDROID_NDK_HOME wajib diisi}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SHIM_SRC="$REPO_ROOT/android/app/src/main/cpp/webforge_node.cpp"

mkdir -p "$OUTPUT_DIR" "$WORK_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
WORK_DIR="$(cd "$WORK_DIR" && pwd)"

TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
if [ ! -d "$TOOLCHAIN" ]; then
  echo "Toolchain NDK linux-x86_64 tidak ditemukan: $TOOLCHAIN" >&2
  exit 1
fi
CXX="$TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API}-clang++"
READELF="$TOOLCHAIN/bin/llvm-readelf"
[ -x "$READELF" ] || READELF="readelf"
if [ ! -x "$CXX" ]; then
  echo "clang++ NDK tidak ditemukan: $CXX" >&2
  exit 1
fi
if [ ! -f "$SHIM_SRC" ]; then
  echo "Shim JNI tidak ditemukan: $SHIM_SRC" >&2
  exit 1
fi

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

verify_elf() {
  local file="$1"
  if [ ! -f "$file" ]; then
    echo "File ELF tidak ditemukan: $file" >&2
    exit 1
  fi
  file "$file" || true
  if ! "$READELF" -h "$file" | grep -q "AArch64"; then
    echo "$file bukan ELF AArch64" >&2
    exit 1
  fi
  "$READELF" -d "$file" | grep NEEDED || true
}

log "Unduh nodejs-mobile v$NODEJS_MOBILE_VERSION"
NODE_ZIP="$WORK_DIR/nodejs-mobile-v${NODEJS_MOBILE_VERSION}-android.zip"
download "https://github.com/nodejs-mobile/nodejs-mobile/releases/download/v${NODEJS_MOBILE_VERSION}/nodejs-mobile-v${NODEJS_MOBILE_VERSION}-android.zip" "$NODE_ZIP"

NODE_EXTRACT="$WORK_DIR/nodejs-mobile"
if [ ! -f "$NODE_EXTRACT/.extracted" ]; then
  rm -rf "$NODE_EXTRACT"
  mkdir -p "$NODE_EXTRACT"
  unzip -q -o "$NODE_ZIP" -d "$NODE_EXTRACT"
  touch "$NODE_EXTRACT/.extracted"
fi

LIBNODE_SRC=""
for cand in \
  "$NODE_EXTRACT/bin/arm64-v8a/libnode.so" \
  "$NODE_EXTRACT/android/arm64-v8a/libnode.so" \
  "$NODE_EXTRACT/arm64-v8a/libnode.so"; do
  if [ -f "$cand" ]; then
    LIBNODE_SRC="$cand"
    break
  fi
done
if [ -z "$LIBNODE_SRC" ]; then
  LIBNODE_SRC="$(find "$NODE_EXTRACT" -path '*arm64-v8a/libnode.so' -type f | head -1)"
fi
if [ -z "$LIBNODE_SRC" ] || [ ! -f "$LIBNODE_SRC" ]; then
  echo "libnode.so arm64-v8a tidak ditemukan di zip nodejs-mobile" >&2
  exit 1
fi

NODE_H="$(find "$NODE_EXTRACT" -path '*/node/node.h' -type f | head -1)"
if [ -z "$NODE_H" ]; then
  NODE_H="$(find "$NODE_EXTRACT" -name node.h -type f | head -1)"
fi
if [ -z "$NODE_H" ] || [ ! -f "$NODE_H" ]; then
  echo "node.h tidak ditemukan di zip nodejs-mobile" >&2
  exit 1
fi
NODE_INCLUDE_DIR="$(dirname "$NODE_H")"
LIBNODE_DIR="$(dirname "$LIBNODE_SRC")"

cp "$LIBNODE_SRC" "$OUTPUT_DIR/libnode.so"
verify_elf "$OUTPUT_DIR/libnode.so"

log "Unduh npm CLI $NPM_VERSION"
download "https://registry.npmjs.org/npm/-/npm-${NPM_VERSION}.tgz" "$OUTPUT_DIR/npm.tgz"
if [ ! -s "$OUTPUT_DIR/npm.tgz" ]; then
  echo "Tarball npm kosong: $OUTPUT_DIR/npm.tgz" >&2
  exit 1
fi

log "Kompilasi shim JNI libwebforge_node.so"
"$CXX" \
  -std=c++17 \
  -fPIC \
  -shared \
  -I"$NODE_INCLUDE_DIR" \
  -I"$(dirname "$NODE_INCLUDE_DIR")" \
  "$SHIM_SRC" \
  -L"$LIBNODE_DIR" \
  -lnode \
  -llog \
  -Wl,-soname,libwebforge_node.so \
  -o "$OUTPUT_DIR/libwebforge_node.so"
verify_elf "$OUTPUT_DIR/libwebforge_node.so"
if ! "$READELF" -d "$OUTPUT_DIR/libwebforge_node.so" | grep -q '\[libnode\.so\]'; then
  echo "libwebforge_node.so tidak bergantung pada libnode.so" >&2
  exit 1
fi

printf '%s\nnpm %s\n' "$NODEJS_MOBILE_VERSION" "$NPM_VERSION" > "$OUTPUT_DIR/node-version.txt"
log "Selesai: $OUTPUT_DIR/libnode.so, $OUTPUT_DIR/libwebforge_node.so, $OUTPUT_DIR/npm.tgz, $OUTPUT_DIR/node-version.txt"
