package com.webforge.mobile

import android.content.Context
import android.util.Log
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap

/**
 * Jembatan JNI ke Rust core (libwebforge_core.so).
 *
 * Tanda tangan external fun di bawah HARUS cocok 1:1 dengan ekspor JNI di
 * core/src/jni_bridge.rs:
 *   Java_com_webforge_mobile_CoreBridge_nativeSetNativeLibDir(JNIEnv, JClass, JString) -> jint
 *   Java_com_webforge_mobile_CoreBridge_nativeSetContext(JNIEnv, JClass, JObject) -> jint
 *   Java_com_webforge_mobile_CoreBridge_nativeStart(JNIEnv, JClass, JString, JString, jint) -> jint
 *   Java_com_webforge_mobile_CoreBridge_nativeStop(JNIEnv, JClass) -> jint
 *   Java_com_webforge_mobile_CoreBridge_nativeGetToken(JNIEnv, JClass) -> jstring
 *
 * @JvmStatic membuat metode menjadi static di kelas CoreBridge sehingga
 * parameter kedua JNI adalah JClass (sesuai signature Rust).
 *
 * Startup cepat (v0.3.1): start() memanggil nativeSetNativeLibDir +
 * nativeSetContext + nativeStart TERLEBIH DAHULU agar listener core segera
 * bind; ekstraksi aset (stdlib Python, aset runtime) berjalan async di
 * thread latar setelah nativeStart sukses. Progresnya jujur lewat
 * [startupState], [assetState], dan ring buffer [startupLogEntries] yang
 * dibaca plugin (getStatus/getStartupLog). Kegagalan ekstraksi tidak
 * disembunyikan: core melaporkan runtime terkait unavailable lewat
 * probe-nya sampai asetnya benar-benar siap.
 */
object CoreBridge {

    const val PORT = 8765
    const val API_BASE = "http://127.0.0.1:$PORT"

    /** Nilai [startupState]: start belum selesai / menunggu start berikutnya. */
    const val STARTUP_STARTING = "starting"

    /** Nilai [startupState]: nativeStart sukses (rc 0); aset boleh masih diekstrak. */
    const val STARTUP_READY = "ready"

    /** Nilai [startupState]: library gagal dimuat atau nativeStart gagal. */
    const val STARTUP_ERROR = "error"

    /** Satu entri ring buffer log startup: timestamp epoch millis + pesan. */
    data class StartupLogEntry(val ts: Long, val message: String)

    private const val TAG = "CoreBridge"
    private const val MAX_STARTUP_LOG_ENTRIES = 200

    /** Urutan aset yang dilaporkan ke plugin lewat [assetState]. */
    private val ASSET_ORDER = listOf(
        "python-stdlib",
        "node-version.txt",
        "python-version.txt",
        "postgres-share.zip",
        "mariadb-share.zip",
        "npm.zip",
        "composer.phar",
        "cacert.pem",
        "icudt74l.dat"
    )

    private val startupLogLock = Any()
    private val startupLog = ArrayDeque<StartupLogEntry>()

    /**
     * Keadaan startup core: "starting" | "ready" | "error".
     * "ready" berarti nativeStart mengembalikan 0 (listener core dimulai);
     * ekstraksi aset async dinilai terpisah lewat [assetState]. Setelah
     * stop() sukses nilainya kembali "starting" (menunggu start berikutnya).
     */
    @Volatile
    var startupState: String = STARTUP_STARTING
        private set

    /**
     * Keadaan per aset: "pending" | "extracting" | "done" |
     * "error: <pesan>". Dibaca plugin sebagai objek JSON
     * { "<assetId>": "<state>", ... }.
     */
    private val assetStates = ConcurrentHashMap<String, String>().apply {
        ASSET_ORDER.forEach { put(it, "pending") }
    }

    /** Generasi ekstraksi aset; thread basi tidak menimpa keadaan generasi baru. */
    @Volatile
    private var extractionGeneration = 0

    @Volatile
    var libraryLoaded: Boolean = false
        private set

    @Volatile
    var loadError: String? = null
        private set

    /** true hanya setelah nativeStart mengembalikan 0. */
    @Volatile
    var started: Boolean = false
        private set

    init {
        try {
            System.loadLibrary("webforge_core")
            libraryLoaded = true
            logStartup("Library native webforge_core dimuat")
        } catch (e: UnsatisfiedLinkError) {
            libraryLoaded = false
            loadError = e.message
            startupState = STARTUP_ERROR
            logStartup("Library native webforge_core gagal dimuat: ${e.message ?: "tidak diketahui"}")
        }
    }

    @JvmStatic
    external fun nativeSetNativeLibDir(dir: String): Int

    @JvmStatic
    external fun nativeSetContext(context: Context): Int

    @JvmStatic
    external fun nativeStart(dataDir: String, workspaceDir: String, port: Int): Int

    @JvmStatic
    external fun nativeStop(): Int

    @JvmStatic
    external fun nativeGetToken(): String

    fun dataDir(context: Context): File = File(context.filesDir, "webforge-data")

    fun workspaceDir(context: Context): File = File(dataDir(context), "workspace")

    /**
     * Mencatat satu kejadian startup/watchdog ke ring buffer (maks
     * [MAX_STARTUP_LOG_ENTRIES] entri, yang tertua dibuang). Dipakai
     * CoreBridge sendiri dan WebForgeService (kejadian watchdog).
     */
    fun logStartup(message: String) {
        val entry = StartupLogEntry(System.currentTimeMillis(), message)
        synchronized(startupLogLock) {
            startupLog.addLast(entry)
            while (startupLog.size > MAX_STARTUP_LOG_ENTRIES) startupLog.removeFirst()
        }
        Log.i(TAG, message)
    }

    /** Snapshot ring buffer log startup, urut lama -> baru. */
    fun startupLogEntries(): List<StartupLogEntry> =
        synchronized(startupLogLock) { startupLog.toList() }

    /** Snapshot keadaan aset per assetId, urut [ASSET_ORDER]. */
    fun assetState(): Map<String, String> {
        val snapshot = LinkedHashMap<String, String>()
        for (id in ASSET_ORDER) snapshot[id] = assetStates[id] ?: "pending"
        return snapshot
    }

    private fun setAssetState(generation: Int, assetId: String, state: String) {
        if (generation != extractionGeneration) return
        assetStates[assetId] = state
    }

    /**
     * Memulai core. Mengembalikan true bila nativeStart sukses (0).
     * Kegagalan library/native dilaporkan apa adanya, tidak disamarkan.
     *
     * Urutan v0.3.1: nativeSetNativeLibDir + nativeSetContext +
     * nativeStart lebih dulu; ekstraksi aset menyusul async di thread
     * latar agar listener core tidak menunggu ekstraksi stdlib Python
     * dan aset runtime pada first run.
     */
    @Synchronized
    fun start(context: Context): Boolean {
        if (!libraryLoaded) {
            startupState = STARTUP_ERROR
            logStartup("Start ditolak: library native tidak termuat: ${loadError ?: "tidak diketahui"}")
            return false
        }
        if (started) {
            logStartup("Start diabaikan: core sudah berjalan")
            return true
        }
        startupState = STARTUP_STARTING
        ASSET_ORDER.forEach { assetStates[it] = "pending" }
        extractionGeneration++
        val generation = extractionGeneration
        logStartup("Memulai core: nativeSetNativeLibDir + nativeSetContext + nativeStart")
        val rc = try {
            // Core memakai nativeLibraryDir untuk menemukan binary bundled.
            val libDirRc = nativeSetNativeLibDir(context.applicationInfo.nativeLibraryDir)
            if (libDirRc != 0) logStartup("nativeSetNativeLibDir rc=$libDirRc (dilanjutkan ke nativeStart)")
            // Core memakai Context untuk memulai NodeService (:node).
            val ctxRc = nativeSetContext(context.applicationContext)
            if (ctxRc != 0) logStartup("nativeSetContext rc=$ctxRc (dilanjutkan ke nativeStart)")
            nativeStart(dataDir(context).absolutePath, workspaceDir(context).absolutePath, PORT)
        } catch (e: UnsatisfiedLinkError) {
            loadError = e.message
            startupState = STARTUP_ERROR
            logStartup("nativeStart gagal (UnsatisfiedLinkError): ${e.message ?: "tidak diketahui"}")
            return false
        }
        logStartup("nativeStart rc=$rc")
        started = rc == 0
        if (!started) {
            startupState = STARTUP_ERROR
            logStartup("Core gagal dimulai (nativeStart rc=$rc)")
            return false
        }
        startupState = STARTUP_READY
        logStartup("Core dimulai (nativeStart rc=0); ekstraksi aset berjalan async di background")
        startAssetExtractionAsync(context.applicationContext, generation)
        return true
    }

    /**
     * Ekstraksi aset di thread latar setelah nativeStart sukses.
     * Kegagalan per aset dicatat ke [assetState] + log startup apa
     * adanya; core akan melaporkan runtime terkait unavailable lewat
     * probe-nya sampai asetnya siap.
     */
    private fun startAssetExtractionAsync(appContext: Context, generation: Int) {
        Thread {
            setAssetState(generation, "python-stdlib", "extracting")
            logStartup("Ekstraksi python-stdlib dimulai")
            val (pythonOk, pythonError) = PythonAssets.ensureStdlibDetailed(appContext)
            if (pythonOk) {
                setAssetState(generation, "python-stdlib", "done")
                logStartup("Ekstraksi python-stdlib selesai")
            } else {
                val message = pythonError ?: "tidak diketahui"
                setAssetState(generation, "python-stdlib", "error: $message")
                logStartup("Ekstraksi python-stdlib gagal: $message")
            }
            AssetCopier.copyRuntimeAssetsDetailed(appContext) { assetId, state ->
                setAssetState(generation, assetId, state)
                when {
                    state == "extracting" -> logStartup("Penyalinan aset $assetId dimulai")
                    state == "done" -> logStartup("Penyalinan aset $assetId selesai")
                    state.startsWith("error") -> logStartup("Penyalinan aset $assetId gagal: ${state.removePrefix("error: ")}")
                }
            }
            logStartup("Ekstraksi aset async selesai (lihat assetState untuk hasil per aset)")
        }.apply {
            name = "webforge-asset-extract"
            isDaemon = true
        }.start()
    }

    @Synchronized
    fun stop(): Boolean {
        extractionGeneration++
        if (!libraryLoaded) {
            started = false
            startupState = STARTUP_ERROR
            logStartup("Stop: library native tidak termuat")
            return true
        }
        val rc = try {
            nativeStop()
        } catch (e: UnsatisfiedLinkError) {
            loadError = e.message
            started = false
            startupState = STARTUP_ERROR
            logStartup("nativeStop gagal (UnsatisfiedLinkError): ${e.message ?: "tidak diketahui"}")
            return false
        }
        started = false
        startupState = STARTUP_STARTING
        logStartup("Core dihentikan (nativeStop rc=$rc)")
        return rc == 0
    }

    /** Token API dari core; string kosong bila core belum berjalan. */
    fun token(): String {
        if (!libraryLoaded || !started) return ""
        return try {
            nativeGetToken()
        } catch (e: UnsatisfiedLinkError) {
            loadError = e.message
            ""
        }
    }

    /**
     * Status nyata: flag started DAN port manajemen benar-benar menerima
     * koneksi TCP. Flag saja tidak cukup untuk mengklaim "running".
     */
    fun isReachable(timeoutMs: Int = 500): Boolean {
        if (!started) return false
        return try {
            Socket().use { socket ->
                socket.connect(InetSocketAddress("127.0.0.1", PORT), timeoutMs)
                true
            }
        } catch (e: Exception) {
            false
        }
    }
}
