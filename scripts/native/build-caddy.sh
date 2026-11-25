#!/usr/bin/env bash
# build-caddy.sh — cross-compile Caddy untuk Android arm64 (WebForge Mobile).
#
# Hasil: $OUTPUT_DIR/libcaddy.so (binary Caddy dikemas sebagai .so agar sah
# dieksekusi dari nativeLibraryDir; patuh W^X Android).
#
# Environment yang diterima:
#   ANDROID_API      API level target (default 26). Diterima untuk
#                    konsistensi antarmuka dengan script native lain;
#                    build Go murni (CGO_ENABLED=0) tidak memakainya.
#                    Binary Go android/arm64 mendukung API >= 21, jadi
#                    aman untuk minSdk 26.
#   ANDROID_NDK_HOME Opsional; tidak dibutuhkan karena CGO_ENABLED=0.
#   OUTPUT_DIR       Folder hasil (default $PWD/native-out).
#   WORK_DIR         Folder kerja/cache (default $PWD/native-work/caddy).
#                    Cache module Go (GOMODCACHE), cache build (GOCACHE),
#                    dan GOPATH semuanya di dalam WORK_DIR agar CI bisa
#                    meng-cache satu folder.
#   CADDY_VERSION    Versi Caddy tanpa awalan "v" (default pinned 2.8.4).
#
# Syarat host: go (>= 1.22 untuk Caddy 2.8.x), git, curl tidak dibutuhkan
# (clone via git), readelf atau llvm-readelf untuk verifikasi ELF.
set -euo pipefail

CADDY_VERSION="${CADDY_VERSION:-2.8.4}"
ANDROID_API="${ANDROID_API:-26}"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/native-out}"
WORK_DIR="${WORK_DIR:-$PWD/native-work/caddy}"

echo "== Build Caddy v${CADDY_VERSION} (android/arm64, API ${ANDROID_API}) =="

if ! command -v go >/dev/null 2>&1; then
  echo "::error::go tidak ditemukan di PATH. Pasang Go >= 1.22 (CI: actions/setup-go)." >&2
  exit 1
fi
if ! command -v git >/dev/null 2>&1; then
  echo "::error::git tidak ditemukan di PATH." >&2
  exit 1
fi
go version

mkdir -p "$OUTPUT_DIR" "$WORK_DIR/src" "$WORK_DIR/gopath" "$WORK_DIR/gomodcache" "$WORK_DIR/gocache"

export GOOS=android
export GOARCH=arm64
export CGO_ENABLED=0
export GOPATH="$WORK_DIR/gopath"
export GOMODCACHE="$WORK_DIR/gomodcache"
export GOCACHE="$WORK_DIR/gocache"
# Unduhan module diverifikasi checksum Go module (go.sum upstream).
export GOFLAGS="-mod=mod"

CADDY_SRC="$WORK_DIR/src/caddy"
if [ ! -d "$CADDY_SRC/.git" ]; then
  echo "-- Clone Caddy v${CADDY_VERSION} dari github.com/caddyserver/caddy --"
  git clone --depth 1 --branch "v${CADDY_VERSION}" \
    https://github.com/caddyserver/caddy.git "$CADDY_SRC"
else
  echo "-- Source Caddy sudah ada di $CADDY_SRC (cache WORK_DIR) --"
fi

TMPBIN="$WORK_DIR/caddy-android-arm64"
echo "-- go build ./cmd/caddy --"
# -s -w adalah strip bawaan Go (buang symbol table + DWARF); strip
# eksternal (llvm-strip) sengaja tidak dipakai agar tidak menyentuh
# section runtime Go.
(
  cd "$CADDY_SRC"
  # CustomVersion + -buildvcs=false: tanpa keduanya `caddy version`
  # menampilkan hash commit VCS hasil clone, bukan versi rilis.
  go build -buildvcs=false -trimpath \
    -ldflags "-s -w -X github.com/caddyserver/caddy/v2.CustomVersion=v${CADDY_VERSION}" \
    -o "$TMPBIN" ./cmd/caddy
)

OUT="$OUTPUT_DIR/libcaddy.so"
cp "$TMPBIN" "$OUT"
chmod +x "$OUT"

echo "-- Verifikasi versi Caddy tertanam --"
go version -m "$OUT" || true
if ! grep -a -q "v${CADDY_VERSION}" "$OUT"; then
  echo "::error::Versi v${CADDY_VERSION} tidak ditemukan pada binary Caddy; CustomVersion gagal tertanam." >&2
  exit 1
fi
echo "OK: CustomVersion v${CADDY_VERSION} tertanam"

echo "-- Verifikasi ELF --"
if command -v file >/dev/null 2>&1; then
  file "$OUT"
fi
READELF=""
if command -v readelf >/dev/null 2>&1; then
  READELF="readelf"
elif [ -n "${ANDROID_NDK_HOME:-}" ]; then
  CANDIDATE=$(ls "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-readelf 2>/dev/null | head -1 || true)
  if [ -n "$CANDIDATE" ]; then
    READELF="$CANDIDATE"
  fi
fi
if [ -z "$READELF" ]; then
  echo "::error::readelf/llvm-readelf tidak ditemukan; verifikasi ELF tidak bisa dilakukan." >&2
  exit 1
fi
"$READELF" -h "$OUT" | grep -E "Class|Type|Machine"
if ! "$READELF" -h "$OUT" | grep "AArch64" >/dev/null; then
  echo "::error::libcaddy.so bukan ELF AArch64." >&2
  exit 1
fi
"$READELF" -l "$OUT" | grep -i "interpreter" || echo "(tanpa PT_INTERP: static PIE)"
echo "DT_NEEDED:"
"$READELF" -d "$OUT" | grep NEEDED || echo "(tidak ada DT_NEEDED)"
for lib in $("$READELF" -d "$OUT" | grep NEEDED | sed -E 's/.*\[(.*)\]/\1/' || true); do
  case "$lib" in
    libc.so|libm.so|libdl.so|liblog.so) ;;
    *)
      echo "::error::Dependensi dinamis tak terduga pada libcaddy.so: $lib" >&2
      exit 1
      ;;
  esac
done

SIZE=$(stat -c%s "$OUT" 2>/dev/null || wc -c < "$OUT")
echo "Selesai: $OUT ($SIZE bytes)"
ls -lh "$OUT"
