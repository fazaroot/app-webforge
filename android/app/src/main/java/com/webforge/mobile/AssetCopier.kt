package com.webforge.mobile

import android.content.Context
import android.util.Log
import java.io.File
import java.io.FileOutputStream
import java.io.IOException

/**
 * Menyalin asset runtime hasil build CI ke data dir core
 * (`webforge-data/assets/`), tempat core Rust membacanya:
 * - node-version.txt, python-version.txt (info versi runtime)
 * - postgres-share.zip, mariadb-share.zip (share database, diekstrak core)
 * - npm.zip (npm CLI, diekstrak core saat instalasi dependensi Node)
 * - composer.phar (Composer 2.11.3, dijalankan core lewat PHP CLI)
 * - cacert.pem (bundle CA Mozilla untuk curl/openssl PHP)
 * - icudt74l.dat (data ICU 74.2 untuk ekstensi intl PHP)
 *
 * Asset yang tidak ada di APK dilewati apa adanya; core akan melaporkan
 * runtime terkait tidak tersedia. Tidak ada klaim asset siap tanpa file.
 * Sejak v0.3.1 dipanggil async oleh CoreBridge setelah nativeStart;
 * penyalinan memakai file .tmp lalu rename agar core tidak membaca
 * file setengah jadi.
 */
object AssetCopier {

    private const val TAG = "AssetCopier"

    private val ASSETS = listOf(
        "node-version.txt",
        "python-version.txt",
        "postgres-share.zip",
        "mariadb-share.zip",
        "npm.zip",
        "composer.phar",
        "cacert.pem",
        "icudt74l.dat"
    )

    fun copyRuntimeAssets(context: Context) {
        copyRuntimeAssetsDetailed(context, null)
    }

    /**
     * Menyalin semua aset dan melaporkan keadaan per aset lewat
     * [onState] ("extracting" -> "done" | "error: <pesan>") serta
     * nilai kembaliannya. Kegagalan per aset tidak menghentikan aset
     * berikutnya dan tidak disembunyikan.
     */
    fun copyRuntimeAssetsDetailed(
        context: Context,
        onState: ((assetId: String, state: String) -> Unit)?
    ): Map<String, String> {
        val results = LinkedHashMap<String, String>()
        val destDir = File(CoreBridge.dataDir(context), "assets")
        if (!destDir.exists() && !destDir.mkdirs()) {
            val message = "Gagal membuat folder ${destDir.absolutePath}"
            Log.e(TAG, message)
            for (name in ASSETS) {
                val state = "error: $message"
                results[name] = state
                onState?.invoke(name, state)
            }
            return results
        }
        for (name in ASSETS) {
            onState?.invoke(name, "extracting")
            try {
                val dest = File(destDir, name)
                val tmp = File(destDir, "$name.tmp")
                context.assets.open(name).use { input ->
                    FileOutputStream(tmp).use { output -> input.copyTo(output) }
                }
                if (dest.exists() && !dest.delete()) {
                    tmp.delete()
                    throw IOException("Gagal mengganti asset lama ${dest.absolutePath}")
                }
                if (!tmp.renameTo(dest)) {
                    tmp.delete()
                    throw IOException("Gagal memindahkan asset $name ke ${dest.absolutePath}")
                }
                Log.i(TAG, "Asset $name disalin (${dest.length()} byte)")
                results[name] = "done"
                onState?.invoke(name, "done")
            } catch (e: Exception) {
                val message = e.message ?: e.javaClass.simpleName
                Log.w(TAG, "Asset $name tidak tersedia di APK: $message")
                val state = "error: $message"
                results[name] = state
                onState?.invoke(name, state)
            }
        }
        return results
    }
}
