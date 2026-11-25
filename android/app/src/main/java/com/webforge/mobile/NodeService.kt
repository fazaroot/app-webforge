package com.webforge.mobile

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.os.Process
import android.util.Log
import org.json.JSONObject
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
 *
 * Mode probe (v0.3.1, kontrak file dengan core): core menulis
 * <data_dir>/node-probe/request.json { nonce } dan skrip probe, lalu
 * memulai service ini lewat jalur yang sudah ada (JNI node_start ->
 * NodeController.start) dengan argumen [probe.js, "--webforge-probe"].
 * Skrip membaca request.json dari foldernya dan menulis result.json
 * { nonce, version: process.version, error: null }. Service ini
 * mendeteksi flag probe pada argumen (atau extra EXTRA_PROBE dari
 * NodeController.startProbe), menjalankan Node apa adanya, dan bila
 * Node tidak menghasilkan result.json dengan nonce yang cocok
 * (library tidak termuat, exit code gagal, exception) service menulis
 * result.json sendiri dengan error persisnya. Setelah hasil tertulis
 * proses :node berhenti sendiri (stopSelf; onDestroy membunuh proses).
 */
class NodeService : Service() {

    @Volatile
    private var worker: Thread? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val args = intent?.getStringArrayExtra(EXTRA_ARGS) ?: emptyArray()
        val probeByFlag = args.contains(NODE_PROBE_FLAG)
        val probeByExtra = intent?.getBooleanExtra(EXTRA_PROBE, false) == true ||
            intent?.hasExtra(EXTRA_PROBE_REQUEST_PATH) == true ||
            intent?.hasExtra(EXTRA_PROBE_RESULT_PATH) == true
        if (probeByFlag || probeByExtra) {
            runProbe(intent, startId, if (probeByFlag) args else null)
            return START_NOT_STICKY
        }

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

    /**
     * Mode probe. [coreArgs] non-null bila core memicu lewat argumen
     * [probe.js, "--webforge-probe"]: skrip core dijalankan apa adanya
     * (ia membaca request.json dari foldernya sendiri). Bila null
     * (extra EXTRA_PROBE), service menulis skrip probe sendiri yang
     * menerima resultPath dan nonce lewat process.argv[2]/[3] (shim
     * C++ menaruh "node" sebagai argv[0]). Di kedua jalur, kegagalan
     * menghasilkan result.json ber-nonce cocok ditulis service sendiri
     * sebagai { nonce, version: null, error: <pesan persis> }.
     */
    private fun runProbe(intent: Intent?, startId: Int, coreArgs: Array<String>?) {
        val defaultProbeDir = File(CoreBridge.dataDir(applicationContext), "node-probe")
        val probeDir = coreArgs?.firstOrNull()?.let { File(it).parentFile } ?: defaultProbeDir
        val requestPath = intent?.getStringExtra(EXTRA_PROBE_REQUEST_PATH)
            ?: File(probeDir, "request.json").absolutePath
        val resultPath = intent?.getStringExtra(EXTRA_PROBE_RESULT_PATH)
            ?: File(probeDir, "result.json").absolutePath
        val logPath = intent?.getStringExtra(EXTRA_LOG_PATH)
            ?: File(probeDir, "probe.log").absolutePath
        val statusPath = intent?.getStringExtra(EXTRA_STATUS_PATH)
            ?: File(probeDir, "probe-status.txt").absolutePath
        val cwdExtra = intent?.getStringExtra(EXTRA_CWD)
        val cwd = if (cwdExtra != null && File(cwdExtra).isDirectory) cwdExtra else probeDir.absolutePath

        try {
            probeDir.mkdirs()
        } catch (e: Exception) {
            Log.w(TAG, "Gagal membuat folder probe ${probeDir.absolutePath}: ${e.message}")
        }
        ensureParentDir(logPath)
        ensureParentDir(statusPath)
        ensureParentDir(resultPath)
        writeStatus(statusPath, "starting")

        var requestError: String? = null
        val nonce: String? = try {
            JSONObject(File(requestPath).readText()).optString("nonce", "").takeIf { it.isNotEmpty() }
        } catch (e: Exception) {
            requestError = "Request probe tidak terbaca ($requestPath): ${e.message ?: e.javaClass.simpleName}"
            null
        }

        // Jalur extra tanpa skrip core membutuhkan nonce untuk skrip
        // argv; tanpa nonce hasil tidak bisa diverifikasi core.
        if (coreArgs == null && nonce == null) {
            val message = requestError ?: "Request probe tidak berisi nonce ($requestPath)"
            Log.e(TAG, message)
            writeProbeResult(resultPath, null, null, message)
            writeStatus(statusPath, "1")
            stopSelf(startId)
            return
        }

        if (!libraryLoaded) {
            val message = "Library webforge_node tidak termuat: ${loadError ?: "tidak diketahui"}"
            Log.e(TAG, message)
            writeProbeResult(resultPath, nonce, null, message)
            writeStatus(statusPath, "127")
            stopSelf(startId)
            return
        }

        val argsToRun: Array<String> = if (coreArgs != null) {
            coreArgs
        } else {
            val scriptFile = File(probeDir, "probe.js")
            try {
                scriptFile.writeText(PROBE_SCRIPT)
            } catch (e: Exception) {
                val message = "Gagal menulis script probe: ${e.message ?: e.javaClass.simpleName}"
                Log.e(TAG, message)
                writeProbeResult(resultPath, nonce, null, message)
                writeStatus(statusPath, "1")
                stopSelf(startId)
                return
            }
            arrayOf(scriptFile.absolutePath, resultPath, nonce ?: "")
        }

        val thread = Thread {
            var failure: String? = null
            val exitCode = try {
                nativeStartNode(argsToRun, cwd, logPath)
            } catch (e: UnsatisfiedLinkError) {
                Log.e(TAG, "nativeStartNode tidak tersedia", e)
                failure = "nativeStartNode tidak tersedia: ${e.message ?: "tidak diketahui"}"
                127
            } catch (e: Exception) {
                Log.e(TAG, "Probe Node gagal dijalankan", e)
                failure = "Probe Node gagal dijalankan: ${e.message ?: e.javaClass.simpleName}"
                1
            }
            if (!probeResultMatches(resultPath, nonce)) {
                val message = failure
                    ?: "Node selesai (exit code $exitCode) tanpa menulis result.json yang cocok; lihat log probe $logPath"
                Log.e(TAG, message)
                writeProbeResult(resultPath, nonce, null, message)
            }
            writeStatus(statusPath, exitCode.toString())
            // Proses :node probe berhenti sendiri setelah hasil tertulis.
            stopSelf(startId)
        }
        thread.name = "webforge-node-probe"
        worker = thread
        thread.start()
    }

    /**
     * true bila result.json ada dan nonce-nya cocok. Bila nonce request
     * tidak terbaca (null), keberadaan result.json saja diterima:
     * skrip core menulis laporan error-nya sendiri dengan nonce null.
     */
    private fun probeResultMatches(resultPath: String, nonce: String?): Boolean {
        return try {
            val result = JSONObject(File(resultPath).readText())
            nonce == null || result.optString("nonce", "") == nonce
        } catch (e: Exception) {
            false
        }
    }

    /** Menulis result.json { nonce, version, error }; null menjadi JSON null. */
    private fun writeProbeResult(resultPath: String, nonce: String?, version: String?, error: String?) {
        try {
            val result = JSONObject()
                .put("nonce", nonce ?: JSONObject.NULL)
                .put("version", version ?: JSONObject.NULL)
                .put("error", error ?: JSONObject.NULL)
            val file = File(resultPath)
            file.parentFile?.mkdirs()
            file.writeText(result.toString())
        } catch (e: Exception) {
            Log.e(TAG, "Gagal menulis hasil probe ke $resultPath", e)
        }
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

        /** Flag argumen penanda mode probe dari core (NODE_PROBE_FLAG). */
        const val NODE_PROBE_FLAG = "--webforge-probe"

        /** true = mode probe lewat extra (lihat runProbe). */
        const val EXTRA_PROBE = "probe"

        /** Path request.json probe; default <data_dir>/node-probe/request.json. */
        const val EXTRA_PROBE_REQUEST_PATH = "probeRequestPath"

        /** Path result.json probe; default <data_dir>/node-probe/result.json. */
        const val EXTRA_PROBE_RESULT_PATH = "probeResultPath"

        private const val TAG = "NodeService"

        /**
         * Skrip probe Node untuk jalur extra: menulis
         * { nonce, version: process.version, error: null } ke
         * resultPath (argv[2]) dengan nonce argv[3]. Jalur core memakai
         * skripnya sendiri (core/src/node_runtime.rs node_probe_script).
         */
        private const val PROBE_SCRIPT = """
const fs = require('fs');
const path = require('path');
const resultPath = process.argv[2];
const nonce = process.argv[3] || '';
function writeResult(version, error) {
  fs.mkdirSync(path.dirname(resultPath), { recursive: true });
  fs.writeFileSync(resultPath, JSON.stringify({ nonce: nonce, version: version, error: error }));
}
try {
  writeResult(process.version, null);
} catch (e) {
  try { writeResult(null, String((e && e.message) || e)); } catch (_) {}
  process.exitCode = 1;
}
"""

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
