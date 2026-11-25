package com.webforge.mobile

import android.content.Context
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket

/**
 * Jembatan JNI ke Rust core (libwebforge_core.so).
 *
 * Tanda tangan external fun di bawah HARUS cocok 1:1 dengan ekspor JNI di
 * core/src/jni_bridge.rs:
 *   Java_com_webforge_mobile_CoreBridge_nativeSetNativeLibDir(JNIEnv, JClass, JString) -> jint
 *   Java_com_webforge_mobile_CoreBridge_nativeStart(JNIEnv, JClass, JString, JString, jint) -> jint
 *   Java_com_webforge_mobile_CoreBridge_nativeStop(JNIEnv, JClass) -> jint
 *   Java_com_webforge_mobile_CoreBridge_nativeGetToken(JNIEnv, JClass) -> jstring
 *
 * @JvmStatic membuat metode menjadi static di kelas CoreBridge sehingga
 * parameter kedua JNI adalah JClass (sesuai signature Rust).
 */
object CoreBridge {

    const val PORT = 8765
    const val API_BASE = "http://127.0.0.1:$PORT"

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
        } catch (e: UnsatisfiedLinkError) {
            libraryLoaded = false
            loadError = e.message
        }
    }

    @JvmStatic
    external fun nativeSetNativeLibDir(dir: String): Int

    @JvmStatic
    external fun nativeStart(dataDir: String, workspaceDir: String, port: Int): Int

    @JvmStatic
    external fun nativeStop(): Int

    @JvmStatic
    external fun nativeGetToken(): String

    fun dataDir(context: Context): File = File(context.filesDir, "webforge-data")

    fun workspaceDir(context: Context): File = File(dataDir(context), "workspace")

    /**
     * Memulai core. Mengembalikan true bila nativeStart sukses (0).
     * Kegagalan library/native dilaporkan apa adanya, tidak disamarkan.
     */
    @Synchronized
    fun start(context: Context): Boolean {
        if (!libraryLoaded) return false
        if (started) return true
        val rc = try {
            // Core memakai nativeLibraryDir untuk menemukan libphp.so.
            nativeSetNativeLibDir(context.applicationInfo.nativeLibraryDir)
            nativeStart(dataDir(context).absolutePath, workspaceDir(context).absolutePath, PORT)
        } catch (e: UnsatisfiedLinkError) {
            loadError = e.message
            return false
        }
        started = rc == 0
        return started
    }

    @Synchronized
    fun stop(): Boolean {
        if (!libraryLoaded) {
            started = false
            return true
        }
        val rc = try {
            nativeStop()
        } catch (e: UnsatisfiedLinkError) {
            loadError = e.message
            started = false
            return false
        }
        started = false
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
