package com.webforge.mobile

import android.content.Context
import android.util.Log
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.util.zip.ZipInputStream

/**
 * Menyiapkan stdlib Python bundled dari assets.
 *
 * Asset yang diharapkan koordinator taruh di CI:
 * - assets/python-stdlib.zip
 * - assets/python-version.txt
 *
 * Hasil di perangkat (di bawah data dir core `webforge-data/`):
 * - python/stdlib.zip
 * - python/lib/python<seri>/ (hasil ekstrak; PYTHONHOME=python/)
 * - python/.stdlib-version (marker)
 *
 * Mengembalikan false apa adanya bila asset tidak ada atau ekstrak gagal;
 * tidak ada klaim stdlib siap tanpa marker dan folder hasil ekstrak.
 * Sejak v0.3.1 dipanggil async oleh CoreBridge setelah nativeStart.
 */
object PythonAssets {

    private const val TAG = "PythonAssets"
    private const val ASSET_ZIP = "python-stdlib.zip"
    private const val ASSET_VERSION = "python-version.txt"
    private const val MARKER_NAME = ".stdlib-version"

    fun ensureStdlib(context: Context): Boolean = ensureStdlibDetailed(context).first

    /**
     * Sama seperti [ensureStdlib] tetapi mengembalikan pesan kegagalan
     * persisnya (second) agar CoreBridge bisa melaporkannya jujur lewat
     * assetState/log startup. second null bila sukses.
     */
    fun ensureStdlibDetailed(context: Context): Pair<Boolean, String?> {
        return try {
            val assetVersion = context.assets.open(ASSET_VERSION).bufferedReader().use {
                it.readText().trim()
            }
            if (assetVersion.isEmpty()) {
                val message = "Asset $ASSET_VERSION kosong"
                Log.e(TAG, message)
                return Pair(false, message)
            }

            val pythonRoot = File(CoreBridge.dataDir(context), "python")
            val zipFile = File(pythonRoot, "stdlib.zip")
            val series = assetVersion.split(".").take(2).joinToString(".")
            val stdlibDir = File(pythonRoot, "lib/python$series")
            val marker = File(pythonRoot, MARKER_NAME)
            val currentVersion = if (marker.isFile) marker.readText().trim() else ""

            if (currentVersion == assetVersion && zipFile.isFile && stdlibDir.isDirectory) {
                Log.i(TAG, "Stdlib Python $assetVersion sudah terpasang")
                return Pair(true, null)
            }

            if (!pythonRoot.exists() && !pythonRoot.mkdirs()) {
                val message = "Gagal membuat folder ${pythonRoot.absolutePath}"
                Log.e(TAG, message)
                return Pair(false, message)
            }

            context.assets.open(ASSET_ZIP).use { input ->
                FileOutputStream(zipFile).use { output -> input.copyTo(output) }
            }
            if (stdlibDir.exists() && !stdlibDir.deleteRecursively()) {
                val message = "Gagal menghapus stdlib lama ${stdlibDir.absolutePath}"
                Log.e(TAG, message)
                return Pair(false, message)
            }
            if (!stdlibDir.mkdirs()) {
                val message = "Gagal membuat folder ${stdlibDir.absolutePath}"
                Log.e(TAG, message)
                return Pair(false, message)
            }
            extractZip(zipFile, stdlibDir)
            marker.writeText(assetVersion)
            Log.i(TAG, "Stdlib Python $assetVersion diekstrak ke ${stdlibDir.absolutePath}")
            Pair(true, null)
        } catch (e: Exception) {
            Log.e(TAG, "Gagal menyiapkan stdlib Python", e)
            Pair(false, e.message ?: e.javaClass.simpleName)
        }
    }

    private fun extractZip(zipFile: File, destDir: File) {
        val canonicalDest = destDir.canonicalPath
        ZipInputStream(FileInputStream(zipFile)).use { zip ->
            var entry = zip.nextEntry
            while (entry != null) {
                val outFile = File(destDir, entry.name)
                val canonicalOut = outFile.canonicalPath
                if (canonicalOut != canonicalDest &&
                    !canonicalOut.startsWith(canonicalDest + File.separator)
                ) {
                    throw IOException("Entri zip di luar folder tujuan: ${entry.name}")
                }
                if (entry.isDirectory) {
                    outFile.mkdirs()
                } else {
                    outFile.parentFile?.mkdirs()
                    FileOutputStream(outFile).use { output -> zip.copyTo(output) }
                }
                zip.closeEntry()
                entry = zip.nextEntry
            }
        }
    }
}
