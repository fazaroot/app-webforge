#!/usr/bin/env bash
# build-nginx.sh — cross-compile Nginx untuk Android arm64 (WebForge Mobile).
#
# Hasil: $OUTPUT_DIR/libnginx.so (binary nginx dikemas sebagai .so agar sah
# dieksekusi dari nativeLibraryDir; patuh W^X Android). PCRE2, zlib, dan
# OpenSSL dibangun statis dari source dengan clang NDK
# aarch64-linux-android<API>, sehingga DT_NEEDED hasil akhir hanya pustaka
# Bionic (libc/libm/libdl/liblog).
#
# Environment yang diterima:
#   ANDROID_NDK_HOME Path NDK. Bila kosong dicari dari $ANDROID_HOME/ndk/*.
#   ANDROID_API      API level target (default 26).
#   OUTPUT_DIR       Folder hasil (default $PWD/native-out).
#   WORK_DIR         Folder kerja (default $PWD/native-work/nginx);
#                    prefix dependensi statis ada di $WORK_DIR/prefix.
#   NGINX_VERSION / PCRE2_VERSION / ZLIB_VERSION / OPENSSL_VERSION
#                    Override versi (default pinned di bawah).
#
# Uji configure cross-compile: build system Nginx menjalankan program uji
# hasil kompilasi (auto/types/sizeof dan uji "run" di auto/feature),
# yang tidak bisa dijalankan saat cross-compile. Script ini menangani
# keduanya secara eksplisit (lihat langkah patch di bawah):
#   - sizeof: program uji dikompilasi dengan compiler host. Ukuran tipe
#     yang diuji Nginx (int, long, long long, void*, size_t, off_t,
#     time_t, sig_atomic_t) identik antara host x86_64 LP64 dan target
#     aarch64 LP64, jadi hasilnya benar untuk target.
#   - feature run: auto/feature dipatch agar menghormati
#     NGX_CROSS_COMPILE=yes — uji run dianggap lolos bila kompilasi
#     lolos, dan uji "value" memakai preset NGX_WITH_<NAMA> (mis.
#     NGX_WITH_NGX_SYS_NERR). Patch gagal keras (exit) bila pola upstream
#     berubah, agar tidak menghasilkan build diam-diam salah.
#   - epoll dipaksa lewat --test-build-epoll (kernel Android mendukung
#     epoll; uji run epoll tidak bisa dijalankan saat cross-compile).
#
# Catatan Android/Bionic:
#   - PCRE2 JIT dimatikan (--disable-jit): JIT membutuhkan memori
#     executable yang umumnya ditolak SELinux aplikasi Android.
#   - crypt(): Bionic tidak menyediakan crypt(); Nginx jatuh ke
#     DES_crypt dari OpenSSL statis untuk http_auth_basic_module.
#     OpenSSL karena itu dibangun tanpa menonaktifkan DES.
#   - Prefix compile-time (/webforge/...) hanya placeholder; runtime
#     memakai -p server_home dan config hasil generate core.
set -euo pipefail

NGINX_VERSION="${NGINX_VERSION:-1.26.2}"
PCRE2_VERSION="${PCRE2_VERSION:-10.44}"
ZLIB_VERSION="${ZLIB_VERSION:-1.3.1}"
OPENSSL_VERSION="${OPENSSL_VERSION:-3.3.2}"
ANDROID_API="${ANDROID_API:-26}"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/native-out}"
WORK_DIR="${WORK_DIR:-$PWD/native-work/nginx}"

echo "== Build Nginx ${NGINX_VERSION} (android/arm64, API ${ANDROID_API}) =="

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

# --- zlib (statis) ---------------------------------------------------------
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

# --- PCRE2 (statis, tanpa JIT) ---------------------------------------------
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

# --- OpenSSL (statis) -------------------------------------------------------
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

# --- Nginx ------------------------------------------------------------------
echo "-- Build Nginx ${NGINX_VERSION} --"
download "https://nginx.org/download/nginx-${NGINX_VERSION}.tar.gz" "$DL/nginx.tar.gz"
rm -rf "$SRC/nginx-${NGINX_VERSION}"
tar -xzf "$DL/nginx.tar.gz" -C "$SRC"
cd "$SRC/nginx-${NGINX_VERSION}"

echo "-- Patch uji cross-compile (sizeof + feature run) --"
if ! command -v python3 >/dev/null 2>&1; then
  echo "::error::python3 dibutuhkan untuk patch cross-compile Nginx." >&2
  exit 1
fi
python3 - <<'PYEOF'
import pathlib
import sys

# 1) auto/types/sizeof: kompilasi program uji sizeof dengan compiler host
#    (ukuran tipe LP64 host == target aarch64).
sizeof_path = pathlib.Path("auto/types/sizeof")
sizeof_text = sizeof_path.read_text()
old = 'ngx_test="$CC'
if old not in sizeof_text:
    sys.exit("Patch sizeof gagal: pola 'ngx_test=\"$CC' tidak ditemukan di auto/types/sizeof (upstream berubah?)")
sizeof_path.write_text(sizeof_text.replace(old, 'ngx_test="cc', 1))
print("patch sizeof: ok")

# 2) auto/feature: hormati NGX_CROSS_COMPILE=yes.
feature_path = pathlib.Path("auto/feature")
feature_text = feature_path.read_text()

anchor = "if [ -x $NGX_AUTOTEST ]; then"
if anchor not in feature_text:
    sys.exit("Patch feature gagal: anchor 'if [ -x $NGX_AUTOTEST ]; then' tidak ditemukan")
cross_block = anchor + """
    if [ ".$NGX_CROSS_COMPILE" = ".yes" ]; then
        NGX_AUTOTEST_EXEC="true"
    else
        NGX_AUTOTEST_EXEC="$NGX_AUTOTEST"
    fi"""
feature_text = feature_text.replace(anchor, cross_block, 1)

run_old = "/bin/sh -c $NGX_AUTOTEST"
if run_old not in feature_text:
    sys.exit("Patch feature gagal: pola run '/bin/sh -c $NGX_AUTOTEST' tidak ditemukan")
feature_text = feature_text.replace(run_old, "/bin/sh -c $NGX_AUTOTEST_EXEC")

value_old = "#define $ngx_feature_name  `$NGX_AUTOTEST`"
if value_old not in feature_text:
    sys.exit("Patch feature gagal: pola value '#define $ngx_feature_name  `$NGX_AUTOTEST`' tidak ditemukan")
value_new = '#define $ngx_feature_name  $(eval "echo \\${NGX_WITH_${ngx_feature_name}:-1}")'
feature_text = feature_text.replace(value_old, value_new)

feature_path.write_text(feature_text)
print("patch feature: ok")
PYEOF

export NGX_CROSS_COMPILE=yes
# Preset uji "value": jumlah errno Linux/Bionic. Hanya dipakai bila jalur
# sys_errlist ikut terkompilasi; Bionic tidak menyediakan sys_errlist,
# jadi nilai ini praktis tidak terpakai — tetap diisi agar define tidak
# kosong. Verifikasi di perangkat tetap wajib.
export NGX_WITH_NGX_SYS_NERR=133
# Bionic API 26 tidak menyediakan crypt.h; jangan klaim NGX_HAVE_CRYPT.
export NGX_WITH_NGX_HAVE_CRYPT=0

# ngx_linux_config.h meng-include <crypt.h> tanpa syarat; Bionic API 26
# tidak menyediakannya dan auth_basic dimatikan, jadi include ini aman dibuang.
sed -i 's|#include <crypt.h>|/* crypt.h tidak tersedia di Bionic API 26 */|' src/os/unix/ngx_linux_config.h
sed -i 's|#include <glob.h>|#include <glob.h>\nint wf_glob(const char *, int, int (*)(const char *, int), glob_t *);\nvoid wf_globfree(glob_t *);|' src/os/unix/ngx_linux_config.h
sed -i 's|n = glob((char \*) gl->pattern|n = wf_glob((char *) gl->pattern|; s|globfree(&gl->pglob)|wf_globfree(\&gl->pglob)|' src/os/unix/ngx_files.c


# Bionic API 26 tidak menyediakan glob()/globfree() (baru API 28), padahal
# Nginx memakainya untuk pola include konfigurasi. Sediakan shim minimal
# berbasis opendir+fnmatch: mendukung pola "dir/*.conf" (direktori literal,
# wildcard pada nama file) yang dipakai include konfigurasi.
cat > "$WORK_DIR/glob_shim.h" <<'EOF'
#ifndef WEBFORGE_GLOB_SHIM_H
#define WEBFORGE_GLOB_SHIM_H
#include <glob.h>
int wf_glob(const char *pattern, int flags, int (*errfunc)(const char *, int), glob_t *pglob);
void wf_globfree(glob_t *pglob);
#endif
EOF
cat > "$WORK_DIR/glob_shim.c" <<'EOF'
#include <glob.h>
#include <dirent.h>
#include <fnmatch.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

#ifndef GLOB_NOMATCH
#define GLOB_NOMATCH 3
#endif

static int cmp_str(const void *a, const void *b) {
    return strcmp(*(char *const *)a, *(char *const *)b);
}

int wf_glob(const char *pattern, int flags, int (*errfunc)(const char *, int), glob_t *pglob) {
    (void)errfunc;
    const char *slash = strrchr(pattern, '/');
    const char *base = slash ? slash + 1 : pattern;
    char dirbuf[1024];
    const char *dir = ".";
    if (slash) {
        size_t dlen = (size_t)(slash - pattern);
        if (dlen == 0) { dir = "/"; }
        else {
            if (dlen >= sizeof(dirbuf)) return GLOB_NOMATCH;
            memcpy(dirbuf, pattern, dlen); dirbuf[dlen] = '\0';
            dir = dirbuf;
        }
    }
    if (strpbrk(dir, "*?[") != NULL) return GLOB_NOMATCH;
    DIR *d = opendir(dir);
    if (!d) return GLOB_NOMATCH;
    size_t cap = 16, n = 0;
    char **list = malloc(cap * sizeof(char *));
    if (!list) { closedir(d); return GLOB_NOMATCH; }
    struct dirent *de;
    while ((de = readdir(d)) != NULL) {
        if (de->d_name[0] == '.' && base[0] != '.') continue;
        if (fnmatch(base, de->d_name, 0) != 0) continue;
        if (n == cap) {
            cap *= 2;
            char **nl = realloc(list, cap * sizeof(char *));
            if (!nl) { closedir(d); return GLOB_NOMATCH; }
            list = nl;
        }
        size_t len = strlen(dir) + 1 + strlen(de->d_name) + 1;
        list[n] = malloc(len);
        if (!list[n]) { closedir(d); return GLOB_NOMATCH; }
        if (slash) snprintf(list[n], len, "%s/%s", dir, de->d_name);
        else snprintf(list[n], len, "%s", de->d_name);
        n++;
    }
    closedir(d);
    if (n == 0) { free(list); return GLOB_NOMATCH; }
    qsort(list, n, sizeof(char *), cmp_str);
    pglob->gl_pathc = n;
    pglob->gl_pathv = list;
    pglob->gl_offs = 0;
    pglob->gl_flags = flags;
    return 0;
}

void wf_globfree(glob_t *pglob) {
    if (!pglob) return;
    for (size_t i = 0; i < pglob->gl_pathc; i++) free(pglob->gl_pathv[i]);
    free(pglob->gl_pathv);
    pglob->gl_pathc = 0;
    pglob->gl_pathv = NULL;
}
EOF
$CC -O2 -fPIC -c "$WORK_DIR/glob_shim.c" -o "$WORK_DIR/glob_shim.o"

if ! ./configure \
  --prefix=/webforge/nginx \
  --sbin-path=/webforge/nginx \
  --conf-path=/webforge/nginx.conf \
  --error-log-path=/webforge/error.log \
  --pid-path=/webforge/nginx.pid \
  --lock-path=/webforge/nginx.lock \
  --http-client-body-temp-path=/webforge/client_body_temp \
  --http-proxy-temp-path=/webforge/proxy_temp \
  --http-fastcgi-temp-path=/webforge/fastcgi_temp \
  --http-uwsgi-temp-path=/webforge/uwsgi_temp \
  --http-scgi-temp-path=/webforge/scgi_temp \
  --with-cc="$CC" \
  --with-cc-opt="-I$PREFIX/include -O2" \
  --with-ld-opt="-L$PREFIX/lib" \
  --with-threads \
  --with-http_realip_module \
  --with-http_gzip_static_module \
  --with-http_stub_status_module \
  --without-http_auth_basic_module; then
  echo "::error::configure Nginx gagal; isi objs/autoconf.err:"
  cat objs/autoconf.err || true
  exit 1
fi


# Tautkan shim glob hanya pada link akhir nginx (bukan uji fitur configure).
sed -i "s|\$(LINK) -o objs/nginx|\$(LINK) -o objs/nginx $WORK_DIR/glob_shim.o|" objs/Makefile

if ! grep -q "NGX_HAVE_EPOLL" objs/ngx_auto_config.h; then
  echo "::warning::NGX_HAVE_EPOLL tidak terdeteksi; Nginx akan memakai poll/select. Periksa log configure."
fi

make -j"$JOBS"

OUT="$OUTPUT_DIR/libnginx.so"
cp objs/nginx "$OUT"
"$STRIP" "$OUT"
chmod +x "$OUT"

echo "-- Verifikasi ELF + DT_NEEDED --"
if command -v file >/dev/null 2>&1; then
  file "$OUT"
fi
"$READELF" -h "$OUT" | grep -E "Class|Type|Machine"
if ! "$READELF" -h "$OUT" | grep "AArch64" >/dev/null; then
  echo "::error::libnginx.so bukan ELF AArch64." >&2
  exit 1
fi
"$READELF" -d "$OUT" | grep NEEDED || echo "(tidak ada DT_NEEDED)"
for lib in $("$READELF" -d "$OUT" | grep NEEDED | sed -E 's/.*\[(.*)\]/\1/' || true); do
  case "$lib" in
    libc.so|libm.so|libdl.so|liblog.so) ;;
    *)
      echo "::error::Dependensi dinamis tak terduga pada libnginx.so: $lib (dependensi harus statis)" >&2
      exit 1
      ;;
  esac
done

SIZE=$(stat -c%s "$OUT" 2>/dev/null || wc -c < "$OUT")
echo "Selesai: $OUT ($SIZE bytes)"
ls -lh "$OUT"
