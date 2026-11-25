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
 */
object PythonAssets {

    private const val TAG = "PythonAssets"
    private const val ASSET_ZIP = "python-stdlib.zip"
    private const val ASSET_VERSION = "python-version.txt"
    private const val MARKER_NAME = ".stdlib-version"

    fun ensureStdlib(context: Context): Boolean {
        return try {
            val assetVersion = context.assets.open(ASSET_VERSION).bufferedReader().use {
                it.readText().trim()
            }
            if (assetVersion.isEmpty()) {
                Log.e(TAG, "Asset $ASSET_VERSION kosong")
                return false
            }

            val pythonRoot = File(CoreBridge.dataDir(context), "python")
            val zipFile = File(pythonRoot, "stdlib.zip")
            val series = assetVersion.split(".").take(2).joinToString(".")
            val stdlibDir = File(pythonRoot, "lib/python$series")
            val marker = File(pythonRoot, MARKER_NAME)
            val currentVersion = if (marker.isFile) marker.readText().trim() else ""

            if (currentVersion == assetVersion && zipFile.isFile && stdlibDir.isDirectory) {
                Log.i(TAG, "Stdlib Python $assetVersion sudah terpasang")
                return true
            }

            if (!pythonRoot.exists() && !pythonRoot.mkdirs()) {
                Log.e(TAG, "Gagal membuat folder ${pythonRoot.absolutePath}")
                return false
            }

            context.assets.open(ASSET_ZIP).use { input ->
                FileOutputStream(zipFile).use { output -> input.copyTo(output) }
            }
            if (stdlibDir.exists() && !stdlibDir.deleteRecursively()) {
                Log.e(TAG, "Gagal menghapus stdlib lama ${stdlibDir.absolutePath}")
                return false
            }
            if (!stdlibDir.mkdirs()) {
                Log.e(TAG, "Gagal membuat folder ${stdlibDir.absolutePath}")
                return false
            }
            extractZip(zipFile, stdlibDir)
            marker.writeText(assetVersion)
            Log.i(TAG, "Stdlib Python $assetVersion diekstrak ke ${stdlibDir.absolutePath}")
            true
        } catch (e: Exception) {
            Log.e(TAG, "Gagal menyiapkan stdlib Python", e)
            false
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
