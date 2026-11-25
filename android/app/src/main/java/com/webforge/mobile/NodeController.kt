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
