package com.webforge.mobile

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.os.Process
import android.util.Log
import java.io.File

/**
 * Service proses :node yang menjalankan Node.js lewat nodejs-mobile.
 *
 * nodejs-mobile tidak bisa di-restart in-process setelah node::Start
 * kembali, jadi service ini berjalan di proses terpisah (:node diatur
 * koordinator di AndroidManifest) dan onDestroy membunuh prosesnya.
 * Status nyata ditulis ke statusPath: "starting" saat mulai dan exit
 * code Node saat selesai. Kesehatan server tetap diperiksa core lewat
 * HTTP, bukan dari klaim service ini.
 */
class NodeService : Service() {

    @Volatile
    private var worker: Thread? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val args = intent?.getStringArrayExtra(EXTRA_ARGS) ?: emptyArray()
        val cwd = intent?.getStringExtra(EXTRA_CWD) ?: filesDir.absolutePath
        val logPath = intent?.getStringExtra(EXTRA_LOG_PATH)
            ?: File(filesDir, "node.log").absolutePath
        val statusPath = intent?.getStringExtra(EXTRA_STATUS_PATH)
            ?: File(filesDir, "node-status.txt").absolutePath

        writeStatus(statusPath, "starting")
        ensureParentDir(logPath)
        ensureParentDir(statusPath)

        if (!File(cwd).isDirectory) {
            Log.e(TAG, "Folder kerja Node tidak ditemukan: $cwd")
            writeStatus(statusPath, "126")
            stopSelf(startId)
            return START_NOT_STICKY
        }
        if (!libraryLoaded) {
            Log.e(TAG, "Library webforge_node tidak termuat: ${loadError ?: "tidak diketahui"}")
            writeStatus(statusPath, "127")
            stopSelf(startId)
            return START_NOT_STICKY
        }

        val thread = Thread {
            val exitCode = try {
                nativeStartNode(args, cwd, logPath)
            } catch (e: UnsatisfiedLinkError) {
                Log.e(TAG, "nativeStartNode tidak tersedia", e)
                127
            } catch (e: Exception) {
                Log.e(TAG, "Node gagal dijalankan", e)
                1
            }
            writeStatus(statusPath, exitCode.toString())
            stopSelf(startId)
        }
        thread.name = "webforge-node"
        worker = thread
        thread.start()
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        super.onDestroy()
        // Stop harus benar-benar menghentikan Node. Membunuh proses :node
        // juga memastikan start berikutnya memakai proses baru, karena
        // node::Start tidak aman dipanggil dua kali dalam proses yang sama.
        Process.killProcess(Process.myPid())
    }

    external fun nativeStartNode(args: Array<String>, cwd: String, logPath: String): Int

    private fun ensureParentDir(path: String) {
        try {
            File(path).parentFile?.mkdirs()
        } catch (e: Exception) {
            Log.w(TAG, "Gagal membuat folder induk untuk $path: ${e.message}")
        }
    }

    private fun writeStatus(statusPath: String, value: String) {
        try {
            val file = File(statusPath)
            file.parentFile?.mkdirs()
            file.writeText(value)
        } catch (e: Exception) {
            Log.e(TAG, "Gagal menulis status Node ke $statusPath", e)
        }
    }

    companion object {
        const val EXTRA_ARGS = "args"
        const val EXTRA_CWD = "cwd"
        const val EXTRA_LOG_PATH = "logPath"
        const val EXTRA_STATUS_PATH = "statusPath"

        private const val TAG = "NodeService"

        @Volatile
        var libraryLoaded: Boolean = false
            private set

        @Volatile
        var loadError: String? = null
            private set

        init {
            try {
                System.loadLibrary("webforge_node")
                libraryLoaded = true
            } catch (e: UnsatisfiedLinkError) {
                libraryLoaded = false
                loadError = e.message
            }
        }
    }
}
