package com.webforge.mobile

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.result.ActivityResult
import androidx.documentfile.provider.DocumentFile
import com.getcapacitor.JSArray
import com.getcapacitor.JSObject
import com.getcapacitor.Plugin
import com.getcapacitor.PluginCall
import com.getcapacitor.PluginMethod
import com.getcapacitor.annotation.ActivityCallback
import com.getcapacitor.annotation.CapacitorPlugin
import org.json.JSONObject
import java.io.File

/**
 * Plugin Capacitor "WebForge" — kontraknya ditentukan frontend
 * (frontend/src/lib/api.ts):
 *   getApiBase(): Promise<{ value: string }>
 *   getToken():   Promise<{ value: string }>
 *   getStatus():  Promise<{
 *     running: boolean,            // TCP 127.0.0.1:8765 benar-benar menerima koneksi
 *     started: boolean,            // nativeStart pernah mengembalikan 0
 *     libraryLoaded: boolean,
 *     port: number,
 *     apiBase: string,
 *     loadError: string | null,    // selalu ada; null bila tidak ada error library/native
 *     startupState: "starting" | "ready" | "error",
 *     assetState: Record<string, string> // assetId -> "pending"|"extracting"|"done"|"error: <pesan>"
 *   }>
 *   getStartupLog(): Promise<{ entries: Array<{ ts: number, message: string }> }>
 *                    // ts = epoch millis; urut lama -> baru; maks 200 entri terakhir
 *   startCore():  Promise<{ started: boolean, apiBase: string, port: number, startupState: string }>
 *   stopCore():   Promise<Record<string, unknown>>
 * Ditambah pickDirectory (SAF) dan openBatterySettings sesuai API_CONTRACT.md,
 * serta metode daya anti-stuck v0.2.0:
 *   setKeepCpuAwake({enabled: boolean}): Promise<{ keepCpuAwake: boolean }>
 *   getPowerStatus(): Promise<{ ignoringBatteryOptimizations: boolean, keepCpuAwake: boolean }>
 *   requestIgnoreBatteryOptimizations(): Promise<{ ignoringBatteryOptimizations: boolean }>
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
            val assetState = JSObject()
            for ((assetId, state) in CoreBridge.assetState()) {
                assetState.put(assetId, state)
            }
            val result = JSObject()
                .put("running", reachable)
                .put("started", CoreBridge.started)
                .put("libraryLoaded", CoreBridge.libraryLoaded)
                .put("port", CoreBridge.PORT)
                .put("apiBase", CoreBridge.API_BASE)
                .put("startupState", CoreBridge.startupState)
                .put("assetState", assetState)
            // loadError selalu disertakan: string pesan error, atau null
            // bila tidak ada error library/native.
            result.put("loadError", CoreBridge.loadError ?: JSONObject.NULL)
            call.resolve(result)
        }.start()
    }

    /**
     * Ring buffer log startup CoreBridge (library dimuat/gagal,
     * nativeStart rc, ekstraksi aset, kejadian watchdog). Hasil:
     * { entries: [{ ts: <epoch millis>, message: string }] },
     * urut lama -> baru.
     */
    @PluginMethod
    fun getStartupLog(call: PluginCall) {
        val entries = JSArray()
        for (entry in CoreBridge.startupLogEntries()) {
            entries.put(
                JSObject()
                    .put("ts", entry.ts)
                    .put("message", entry.message)
            )
        }
        call.resolve(JSObject().put("entries", entries))
    }

    @PluginMethod
    fun startCore(call: PluginCall) {
        WebForgeService.start(context)
        call.resolve(
            JSObject()
                .put("started", true)
                .put("apiBase", CoreBridge.API_BASE)
                .put("port", CoreBridge.PORT)
                .put("startupState", CoreBridge.startupState)
        )
    }

    @PluginMethod
    fun stopCore(call: PluginCall) {
        WebForgeService.stop(context)
        call.resolve(JSObject().put("stopped", true))
    }

    /**
     * Toggle "jaga CPU tetap bangun": disimpan di SharedPreferences dan
     * diterapkan segera — service yang hidup menilai ulang WakeLock-nya
     * sekarang juga. WakeLock sendiri tetap hanya dipegang bila core
     * berjalan dan ada server berstatus running (keputusan di service).
     */
    @PluginMethod
    fun setKeepCpuAwake(call: PluginCall) {
        val enabled: Boolean? = call.getBoolean("enabled")
        if (enabled == null) {
            call.reject("Parameter 'enabled' wajib berupa boolean")
            return
        }
        WebForgeService.writeKeepCpuAwake(context, enabled)
        WebForgeService.requestWakeLockReevaluation()
        call.resolve(JSObject().put("keepCpuAwake", enabled))
    }

    /** Status daya nyata: pengecualian baterai dari PowerManager + toggle. */
    @PluginMethod
    fun getPowerStatus(call: PluginCall) {
        val powerManager = context.getSystemService(Context.POWER_SERVICE) as PowerManager
        call.resolve(
            JSObject()
                .put(
                    "ignoringBatteryOptimizations",
                    powerManager.isIgnoringBatteryOptimizations(context.packageName)
                )
                .put("keepCpuAwake", WebForgeService.readKeepCpuAwake(context))
        )
    }

    /**
     * Meminta pengecualian optimasi baterai lewat dialog sistem untuk
     * package ini. Hasilnya adalah status pada saat pemanggilan ini juga —
     * jujur bisa masih false karena pengguna belum menjawab dialog.
     */
    @PluginMethod
    fun requestIgnoreBatteryOptimizations(call: PluginCall) {
        try {
            val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
                data = Uri.parse("package:${context.packageName}")
            }
            activity.startActivity(intent)
            val powerManager = context.getSystemService(Context.POWER_SERVICE) as PowerManager
            call.resolve(
                JSObject().put(
                    "ignoringBatteryOptimizations",
                    powerManager.isIgnoringBatteryOptimizations(context.packageName)
                )
            )
        } catch (e: Exception) {
            call.reject(
                "Gagal meminta pengecualian optimasi baterai: ${e.message ?: e.javaClass.simpleName}"
            )
        }
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
