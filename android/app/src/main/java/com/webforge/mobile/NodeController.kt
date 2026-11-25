package com.webforge.mobile

import android.content.Context
import android.content.Intent
import android.util.Log
import java.io.File

/**
 * Pengendali NodeService dari proses utama.
 *
 * Objek ini hanya mengirim Intent start/stop dan menulis status file.
 * Ia tidak mengklaim proses Node hidup: core memeriksa kesehatan nyata
 * lewat HTTP health check dan statusPath yang ditulis NodeService.
 *
 * Signature start(Context, Array<String>, String, String, String)
 * dipanggil core lewat JNI (jni_bridge.rs node_start) dan TIDAK boleh
 * berubah. Probe Node dipicu core lewat jalur start yang sama dengan
 * argumen [probe.js, "--webforge-probe"] (dideteksi NodeService);
 * startProbe di bawah adalah jalur alternatif yang mengirim Intent
 * yang sama dengan extra probe (NodeService.EXTRA_PROBE).
 */
object NodeController {

    private const val TAG = "NodeController"

    @Volatile
    private var lastStatusPath: String? = null

    fun start(
        context: Context,
        args: Array<String>,
        cwd: String,
        logPath: String,
        statusPath: String
    ): Boolean {
        lastStatusPath = statusPath
        return try {
            val intent = Intent(context, NodeService::class.java)
                .putExtra(NodeService.EXTRA_ARGS, args)
                .putExtra(NodeService.EXTRA_CWD, cwd)
                .putExtra(NodeService.EXTRA_LOG_PATH, logPath)
                .putExtra(NodeService.EXTRA_STATUS_PATH, statusPath)
            context.startService(intent)
            true
        } catch (e: Exception) {
            Log.e(TAG, "Gagal memulai NodeService", e)
            false
        }
    }

    /**
     * Memicu probe Node: core sudah menulis
     * <data_dir>/node-probe/request.json { nonce } sebelum memanggil
     * ini; NodeService menjalankan script probe yang menulis
     * <data_dir>/node-probe/result.json { nonce, version, error } lalu
     * berhenti sendiri. Mengembalikan true bila Intent terkirim.
     */
    fun startProbe(context: Context): Boolean {
        val probeDir = File(CoreBridge.dataDir(context), "node-probe")
        return startProbe(
            context,
            File(probeDir, "request.json").absolutePath,
            File(probeDir, "result.json").absolutePath
        )
    }

    /** Varian startProbe dengan path request/result eksplisit. */
    fun startProbe(context: Context, requestPath: String, resultPath: String): Boolean {
        val probeDir = File(resultPath).parentFile
            ?: File(CoreBridge.dataDir(context), "node-probe")
        val statusPath = File(probeDir, "probe-status.txt").absolutePath
        lastStatusPath = statusPath
        return try {
            val intent = Intent(context, NodeService::class.java)
                .putExtra(NodeService.EXTRA_PROBE, true)
                .putExtra(NodeService.EXTRA_PROBE_REQUEST_PATH, requestPath)
                .putExtra(NodeService.EXTRA_PROBE_RESULT_PATH, resultPath)
                .putExtra(NodeService.EXTRA_CWD, probeDir.absolutePath)
                .putExtra(NodeService.EXTRA_LOG_PATH, File(probeDir, "probe.log").absolutePath)
                .putExtra(NodeService.EXTRA_STATUS_PATH, statusPath)
            context.startService(intent)
            true
        } catch (e: Exception) {
            Log.e(TAG, "Gagal memulai probe NodeService", e)
            false
        }
    }

    fun stop(context: Context): Boolean {
        val statusPath = lastStatusPath
        return if (statusPath != null) {
            stop(context, statusPath)
        } else {
            stopService(context)
        }
    }

    fun stop(context: Context, statusPath: String): Boolean {
        writeStopped(statusPath)
        return stopService(context)
    }

    private fun stopService(context: Context): Boolean {
        return try {
            context.stopService(Intent(context, NodeService::class.java))
        } catch (e: Exception) {
            Log.e(TAG, "Gagal menghentikan NodeService", e)
            false
        }
    }

    private fun writeStopped(statusPath: String) {
        try {
            val file = File(statusPath)
            file.parentFile?.mkdirs()
            file.writeText("stopped")
        } catch (e: Exception) {
            Log.e(TAG, "Gagal menulis status stopped ke $statusPath", e)
        }
    }
}
