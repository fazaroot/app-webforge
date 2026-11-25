package com.webforge.mobile

import android.content.Context
import android.util.Log
import java.io.File
import java.io.FileOutputStream

/**
 * Menyalin asset runtime hasil build CI ke data dir core
 * (`webforge-data/assets/`), tempat core Rust membacanya:
 * - node-version.txt, python-version.txt (info versi runtime)
 * - postgres-share.zip, mariadb-share.zip (share database, diekstrak core)
 * - npm.zip (npm CLI, diekstrak core saat instalasi dependensi Node)
 *
 * Asset yang tidak ada di APK dilewati apa adanya; core akan melaporkan
 * runtime terkait tidak tersedia. Tidak ada klaim asset siap tanpa file.
 */
object AssetCopier {

    private const val TAG = "AssetCopier"

    private val ASSETS = listOf(
        "node-version.txt",
        "python-version.txt",
        "postgres-share.zip",
        "mariadb-share.zip",
        "npm.zip"
    )

    fun copyRuntimeAssets(context: Context) {
        val destDir = File(CoreBridge.dataDir(context), "assets")
        if (!destDir.exists() && !destDir.mkdirs()) {
            Log.e(TAG, "Gagal membuat folder ${destDir.absolutePath}")
            return
        }
        for (name in ASSETS) {
            try {
                val dest = File(destDir, name)
                context.assets.open(name).use { input ->
                    FileOutputStream(dest).use { output -> input.copyTo(output) }
                }
                Log.i(TAG, "Asset $name disalin (${dest.length()} byte)")
            } catch (e: Exception) {
                Log.w(TAG, "Asset $name tidak tersedia di APK: ${e.message}")
            }
        }
    }
}
