package com.webforge.mobile

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.result.ActivityResult
import androidx.documentfile.provider.DocumentFile
import com.getcapacitor.JSObject
import com.getcapacitor.Plugin
import com.getcapacitor.PluginCall
import com.getcapacitor.PluginMethod
import com.getcapacitor.annotation.ActivityCallback
import com.getcapacitor.annotation.CapacitorPlugin
import java.io.File

/**
 * Plugin Capacitor "WebForge" — kontraknya ditentukan frontend
 * (frontend/src/lib/api.ts):
 *   getApiBase(): Promise<{ value: string }>
 *   getToken():   Promise<{ value: string }>
 *   getStatus():  Promise<Record<string, unknown>>
 *   startCore():  Promise<Record<string, unknown>>
 *   stopCore():   Promise<Record<string, unknown>>
 * Ditambah pickDirectory (SAF) dan openBatterySettings sesuai API_CONTRACT.md.
 */
@CapacitorPlugin(name = "WebForge")
class WebForgePlugin : Plugin() {

    @PluginMethod
    fun getApiBase(call: PluginCall) {
        call.resolve(JSObject().put("value", CoreBridge.API_BASE))
    }

    @PluginMethod
    fun getToken(call: PluginCall) {
        // Token hanya ada setelah core berjalan; kosong = jujur belum jalan.
        call.resolve(JSObject().put("value", CoreBridge.token()))
    }

    @PluginMethod
    fun getStatus(call: PluginCall) {
        // Cek TCP nyata di thread latar; flag started saja tidak cukup.
        Thread {
            val reachable = CoreBridge.isReachable()
            val result = JSObject()
                .put("running", reachable)
                .put("started", CoreBridge.started)
                .put("libraryLoaded", CoreBridge.libraryLoaded)
                .put("port", CoreBridge.PORT)
                .put("apiBase", CoreBridge.API_BASE)
            CoreBridge.loadError?.let { result.put("loadError", it) }
            call.resolve(result)
        }.start()
    }

    @PluginMethod
    fun startCore(call: PluginCall) {
        WebForgeService.start(context)
        call.resolve(
            JSObject()
                .put("started", true)
                .put("apiBase", CoreBridge.API_BASE)
                .put("port", CoreBridge.PORT)
        )
    }

    @PluginMethod
    fun stopCore(call: PluginCall) {
        WebForgeService.stop(context)
        call.resolve(JSObject().put("stopped", true))
    }

    /**
     * SAF: pengguna memilih pohon dokumen (ACTION_OPEN_DOCUMENT_TREE),
     * lalu seluruh isinya disalin ke workspace app-managed karena server
     * membutuhkan path filesystem nyata. Hasil: path tujuan + jumlah file.
     * Kegagalan dilaporkan apa adanya lewat reject.
     */
    @PluginMethod
    fun pickDirectory(call: PluginCall) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
            )
        }
        startActivityForResult(call, intent, "handlePickDirectoryResult")
    }

    @ActivityCallback
    fun handlePickDirectoryResult(call: PluginCall, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) {
            call.reject("Pemilihan folder dibatalkan pengguna")
            return
        }
        val treeUri: Uri? = result.data?.data
        if (treeUri == null) {
            call.reject("Folder terpilih tidak mengembalikan URI")
            return
        }
        // Simpan izin persisten bila penyedia mengizinkannya; kegagalan di
        // sini tidak menggagalkan penyalinan yang sedang berjalan.
        try {
            val flags = (result.data?.flags ?: 0) and
                (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            if (flags != 0) {
                context.contentResolver.takePersistableUriPermission(treeUri, flags)
            }
        } catch (e: SecurityException) {
            // Izin persisten tidak tersedia; salinan tetap dilanjutkan.
        }

        Thread {
            try {
                val source = DocumentFile.fromTreeUri(context, treeUri)
                    ?: throw IllegalStateException("URI folder tidak dapat dibuka")
                val rawName = source.name?.takeIf { it.isNotBlank() } ?: "proyek-impor"
                val destRoot = File(CoreBridge.workspaceDir(context), "imported")
                val dest = uniqueDir(destRoot, sanitizeDirName(rawName))
                var fileCount = 0
                copyTree(source, dest) { fileCount++ }
                call.resolve(
                    JSObject()
                        .put("path", dest.absolutePath)
                        .put("name", rawName)
                        .put("fileCount", fileCount)
                )
            } catch (e: Exception) {
                call.reject("Gagal menyalin folder: ${e.message ?: e.javaClass.simpleName}")
            }
        }.start()
    }

    private fun copyTree(source: DocumentFile, dest: File, onFileCopied: () -> Unit) {
        if (!dest.exists() && !dest.mkdirs()) {
            throw IllegalStateException("Tidak dapat membuat folder tujuan: ${dest.absolutePath}")
        }
        for (child in source.listFiles()) {
            val name = child.name ?: continue
            val target = File(dest, name)
            if (child.isDirectory) {
                copyTree(child, target, onFileCopied)
            } else if (child.isFile) {
                val input = context.contentResolver.openInputStream(child.uri)
                    ?: throw IllegalStateException("Tidak dapat membaca file: $name")
                input.use { stream ->
                    target.outputStream().use { out -> stream.copyTo(out) }
                }
                onFileCopied()
            }
        }
    }

    private fun uniqueDir(parent: File, baseName: String): File {
        var candidate = File(parent, baseName)
        var suffix = 2
        while (candidate.exists()) {
            candidate = File(parent, "$baseName-$suffix")
            suffix++
        }
        return candidate
    }

    private fun sanitizeDirName(name: String): String {
        val cleaned = name.replace(Regex("[^A-Za-z0-9._-]"), "-").trim('-', '.')
        return cleaned.ifBlank { "proyek-impor" }
    }

    /** Membuka pengaturan pengecualian optimasi baterai untuk aplikasi ini. */
    @PluginMethod
    fun openBatterySettings(call: PluginCall) {
        try {
            val direct = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
                data = Uri.parse("package:${context.packageName}")
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M &&
                direct.resolveActivity(context.packageManager) != null
            ) {
                activity.startActivity(direct)
            } else {
                activity.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
            }
            call.resolve(JSObject().put("opened", true))
        } catch (e: Exception) {
            call.reject("Gagal membuka pengaturan baterai: ${e.message ?: e.javaClass.simpleName}")
        }
    }
}
