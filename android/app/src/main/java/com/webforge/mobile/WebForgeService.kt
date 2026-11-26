package com.webforge.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.InetSocketAddress
import java.net.Socket
import java.net.URL
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit

/**
 * Foreground service yang menjaga Rust core tetap hidup selama pengguna
 * membutuhkannya. Service ini pemilik lifecycle core: start/stop core
 * selalu lewat sini (plugin dan notifikasi memicu aksi yang sama).
 *
 * Tipe FGS: specialUse (targetSdk 35 mewajibkan tipe eksplisit).
 * Subtype dinyatakan jujur di AndroidManifest: server pengembangan lokal
 * yang berjalan di perangkat atas permintaan pengguna.
 *
 * Anti-stuck (v0.2.0): foreground service saja tidak mencegah CPU suspend
 * saat layar mati — task Tokio in-process ikut membeku. Karena itu service
 * ini memegang PARTIAL_WAKE_LOCK hanya selama syaratnya terpenuhi secara
 * nyata (toggle keepCpuAwake aktif, core berjalan, dan minimal satu server
 * berstatus "running" menurut management API), serta menjalankan watchdog
 * yang memeriksa kesehatan core dan memulai ulangnya bila tidak merespons.
 */
class WebForgeService : Service() {

    /** WakeLock parsial; dibuat sekali di onCreate, tidak reference-counted. */
    private var wakeLock: PowerManager.WakeLock? = null

    /**
     * Keadaan server aktif terakhir yang diketahui dari cek /api/servers
     * yang berhasil. Dipakai saat cek berikutnya gagal sesaat agar
     * WakeLock tidak lepas hanya karena satu kegagalan transien.
     */
    private var lastKnownActiveServer = false

    /** Penjadwal pemeriksaan WakeLock (15 dtk) dan watchdog (30 dtk). */
    private var scheduler: ScheduledExecutorService? = null

    /** true setelah STOP/destroy: watchdog dan evaluator harus berhenti. */
    @Volatile
    private var stopping = false

    /** Jumlah kegagalan restart berturut-turut pada insiden watchdog saat ini. */
    @Volatile
    private var consecutiveRestartFailures = 0

    /** true setelah 3 kali restart gagal: watchdog berhenti mencoba. */
    @Volatile
    private var watchdogGaveUp = false

    /** true bila notifikasi terakhir menampilkan error watchdog. */
    @Volatile
    private var watchdogErrorShown = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
        val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = powerManager.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKELOCK_TAG).apply {
            // Tidak reference-counted: acquire ulang memperpanjang timeout,
            // release tunggal melepasnya.
            setReferenceCounted(false)
        }
        activeInstance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopping = true
                stopSchedulers()
                releaseWakeLock()
                stopCoreAndSelf()
                return START_NOT_STICKY
            }
            else -> {
                // ACTION_START atau restart oleh sistem (intent null dengan
                // START_STICKY): startForeground dipanggil segera di sini,
                // sebelum pekerjaan berat apa pun (batas 5 detik Android),
                // lalu core dimulai di thread latar.
                stopping = false
                // Start eksplisit memberi watchdog kesempatan baru.
                consecutiveRestartFailures = 0
                watchdogGaveUp = false
                startForegroundCompat(buildNotification(running = CoreBridge.started, error = null))
                ensureSchedulersStarted()
                Thread {
                    val ok = CoreBridge.start(applicationContext)
                    val error = when {
                        ok -> null
                        !CoreBridge.libraryLoaded ->
                            "Library native tidak termuat: ${CoreBridge.loadError ?: "tidak diketahui"}"
                        else -> "Core gagal dimulai (nativeStart mengembalikan error)"
                    }
                    if (ok) watchdogErrorShown = false
                    updateNotification(buildNotification(running = ok, error = error))
                    // Nilai WakeLock segera setelah core hidup, tidak menunggu
                    // siklus 15 detik berikutnya.
                    evaluateWakeLock()
                }.start()
                return START_STICKY
            }
        }
    }

    override fun onDestroy() {
        stopping = true
        stopSchedulers()
        releaseWakeLock()
        if (activeInstance === this) activeInstance = null
        // Graceful shutdown: hentikan core sebelum proses service mati.
        CoreBridge.stop()
        super.onDestroy()
    }

    private fun stopCoreAndSelf() {
        Thread {
            CoreBridge.stop()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }.start()
    }

    // ------------------------------------------------------------------
    // Penjadwalan pemeriksaan berkala
    // ------------------------------------------------------------------

    private fun ensureSchedulersStarted() {
        val existing = scheduler
        if (existing != null && !existing.isShutdown) return
        val created = Executors.newScheduledThreadPool(2)
        scheduler = created
        // scheduleWithFixedDelay membatalkan siklus berikutnya bila tugas
        // melempar exception, jadi kedua tugas dibungkus try-catch penuh.
        created.scheduleWithFixedDelay(
            { safeEvaluateWakeLock() },
            WAKELOCK_CHECK_SECONDS, WAKELOCK_CHECK_SECONDS, TimeUnit.SECONDS
        )
        created.scheduleWithFixedDelay(
            { safeWatchdogCheck() },
            WATCHDOG_CHECK_SECONDS, WATCHDOG_CHECK_SECONDS, TimeUnit.SECONDS
        )
    }

    private fun stopSchedulers() {
        scheduler?.shutdownNow()
        scheduler = null
    }

    private fun safeEvaluateWakeLock() {
        try {
            evaluateWakeLock()
        } catch (e: Exception) {
            Log.e(TAG, "Evaluasi WakeLock gagal", e)
        }
    }

    private fun safeWatchdogCheck() {
        try {
            watchdogCheck()
        } catch (e: Exception) {
            Log.e(TAG, "Pemeriksaan watchdog gagal", e)
        }
    }

    /** Menjalankan evaluasi WakeLock di thread latar (dipanggil plugin). */
    fun evaluateWakeLockAsync() {
        val current = scheduler
        if (current != null && !current.isShutdown) {
            current.execute { safeEvaluateWakeLock() }
        } else {
            Thread { safeEvaluateWakeLock() }.start()
        }
    }

    // ------------------------------------------------------------------
    // WakeLock: hanya bila toggle aktif + core berjalan + ada server running
    // ------------------------------------------------------------------

    /**
     * Memutuskan WakeLock berdasarkan keadaan nyata saat ini. Harus
     * dipanggil dari thread latar (melakukan HTTP ke management API).
     * WakeLock di-acquire dengan timeout pengaman 10 menit dan diperpanjang
     * pada pemeriksaan berikutnya selama syarat masih terpenuhi, sehingga
     * tidak pernah bocor selamanya bila service mati tanpa onDestroy.
     */
    private fun evaluateWakeLock() {
        if (stopping) {
            releaseWakeLock()
            return
        }
        if (!readKeepCpuAwake(applicationContext)) {
            releaseWakeLock()
            return
        }
        val active = activeServerState()
        val shouldHold = when {
            active != null -> {
                lastKnownActiveServer = active
                active
            }
            watchdogGaveUp -> false
            else -> {
                if (lastKnownActiveServer) {
                    Log.w(TAG, "Cek /api/servers gagal sesaat; WakeLock dipertahankan dari keadaan terakhir")
                }
                lastKnownActiveServer
            }
        }
        // Periksa ulang stopping: STOP bisa tiba selama pemeriksaan HTTP.
        if (shouldHold && !stopping) {
            wakeLock?.acquire(WAKELOCK_TIMEOUT_MS)
        } else {
            releaseWakeLock()
        }
    }

    private fun releaseWakeLock() {
        try {
            val lock = wakeLock
            if (lock != null && lock.isHeld) lock.release()
        } catch (e: RuntimeException) {
            Log.w(TAG, "Gagal melepas WakeLock", e)
        }
    }

    /**
     * Memeriksa GET /api/servers pada management API (127.0.0.1:8765)
     * dengan header X-API-Key berisi token core (CoreBridge.token()
     * meneruskan nativeGetToken()). Mengembalikan true bila ada server
     * starting/running/degraded, false bila cek berhasil dan tidak ada
     * server aktif, dan null bila keadaan tidak diketahui (core belum
     * siap, token kosong, HTTP gagal, atau respons tidak terbaca).
     * Nilai null tidak boleh melepas WakeLock sendirian.
     */
    private fun activeServerState(): Boolean? {
        if (!CoreBridge.started) return null
        val token = CoreBridge.token()
        if (token.isEmpty()) return null
        return try {
            val connection = (URL("${CoreBridge.API_BASE}/api/servers").openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = HTTP_TIMEOUT_MS
                readTimeout = HTTP_TIMEOUT_MS
                setRequestProperty("X-API-Key", token)
            }
            try {
                if (connection.responseCode != HttpURLConnection.HTTP_OK) {
                    null
                } else {
                    val body = connection.inputStream.bufferedReader().use { it.readText() }
                    val servers = JSONArray(body)
                    var active = false
                    for (i in 0 until servers.length()) {
                        val status = servers.getJSONObject(i).optString("status")
                        if (status == "starting" || status == "running" || status == "degraded") {
                            active = true
                            break
                        }
                    }
                    active
                }
            } finally {
                connection.disconnect()
            }
        } catch (e: Exception) {
            Log.w(TAG, "Gagal memeriksa /api/servers: ${e.message}")
            null
        }
    }

    // ------------------------------------------------------------------
    // Watchdog: TCP + HTTP health, restart core maks 3 kali
    // ------------------------------------------------------------------

    private fun watchdogCheck() {
        if (stopping) return
        if (isCoreHealthy()) {
            if (consecutiveRestartFailures > 0 || watchdogGaveUp || watchdogErrorShown) {
                Log.i(TAG, "Core kembali sehat; counter watchdog direset")
                CoreBridge.logStartup("Watchdog: core kembali sehat; counter watchdog direset")
            }
            consecutiveRestartFailures = 0
            watchdogGaveUp = false
            if (watchdogErrorShown) {
                watchdogErrorShown = false
                updateNotification(buildNotification(running = true, error = null))
            }
            return
        }
        if (watchdogGaveUp) {
            // Sudah 3 kali gagal: berhenti mencoba sampai core sehat lagi
            // atau pengguna menekan Start.
            return
        }
        restartCoreWithRetries()
    }

    /**
     * Kesehatan nyata: TCP connect ke 127.0.0.1:8765 (timeout 2 detik),
     * lalu HTTP GET /api/health harus menjawab 200 dengan status "ok"
     * (endpoint health tidak butuh token menurut API_CONTRACT.md).
     */
    private fun isCoreHealthy(): Boolean {
        try {
            Socket().use { socket ->
                socket.connect(InetSocketAddress("127.0.0.1", CoreBridge.PORT), HTTP_TIMEOUT_MS)
            }
        } catch (e: Exception) {
            return false
        }
        return try {
            val connection = (URL("${CoreBridge.API_BASE}/api/health").openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = HTTP_TIMEOUT_MS
                readTimeout = HTTP_TIMEOUT_MS
            }
            try {
                if (connection.responseCode != HttpURLConnection.HTTP_OK) {
                    false
                } else {
                    val body = connection.inputStream.bufferedReader().use { it.readText() }
                    JSONObject(body).optString("status") == "ok"
                }
            } finally {
                connection.disconnect()
            }
        } catch (e: Exception) {
            false
        }
    }

    /**
     * Memulai ulang core lewat jalur yang sudah ada (CoreBridge.stop()
     * memanggil nativeStop, CoreBridge.start() memanggil nativeStart)
     * maks 3 kali berturut-turut dengan jeda 5 detik antar percobaan.
     * Jeda itu juga memberi waktu core selesai bind sebelum health check.
     */
    private fun restartCoreWithRetries() {
        for (attempt in 1..MAX_RESTART_ATTEMPTS) {
            if (stopping) return
            Log.w(TAG, "Core tidak merespons, memulai ulang (percobaan $attempt/$MAX_RESTART_ATTEMPTS)")
            CoreBridge.logStartup("Watchdog: core tidak merespons, memulai ulang (percobaan $attempt/$MAX_RESTART_ATTEMPTS)")
            watchdogErrorShown = true
            updateNotification(
                buildNotification(
                    running = false,
                    error = "Core tidak merespons, memulai ulang (percobaan $attempt/$MAX_RESTART_ATTEMPTS)…"
                )
            )
            CoreBridge.stop()
            val startedOk = CoreBridge.start(applicationContext)
            try {
                Thread.sleep(RESTART_DELAY_MS)
            } catch (e: InterruptedException) {
                Thread.currentThread().interrupt()
                return
            }
            if (stopping) return
            if (startedOk && isCoreHealthy()) {
                consecutiveRestartFailures = 0
                watchdogGaveUp = false
                watchdogErrorShown = false
                Log.i(TAG, "Core berhasil dimulai ulang pada percobaan $attempt")
                CoreBridge.logStartup("Watchdog: core berhasil dimulai ulang pada percobaan $attempt")
                updateNotification(buildNotification(running = true, error = null))
                evaluateWakeLock()
                return
            }
            consecutiveRestartFailures = attempt
            Log.w(TAG, "Percobaan restart core ke-$attempt gagal")
            CoreBridge.logStartup("Watchdog: percobaan restart core ke-$attempt gagal")
        }
        watchdogGaveUp = true
        Log.e(TAG, "Core gagal dimulai ulang setelah $MAX_RESTART_ATTEMPTS percobaan; watchdog berhenti mencoba")
        CoreBridge.logStartup("Watchdog: core gagal dimulai ulang setelah $MAX_RESTART_ATTEMPTS percobaan; watchdog berhenti mencoba")
        updateNotification(
            buildNotification(
                running = false,
                error = "Core gagal dimulai ulang. Buka aplikasi untuk memeriksa."
            )
        )
        releaseWakeLock()
    }

    // ------------------------------------------------------------------
    // Notifikasi
    // ------------------------------------------------------------------

    private fun startForegroundCompat(notification: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun updateNotification(notification: Notification) {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, notification)
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "WebForge Core",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Status server pengembangan lokal WebForge Mobile"
                setShowBadge(false)
            }
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(running: Boolean, error: String?): Notification {
        val openIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
        }
        val openPending = PendingIntent.getActivity(
            this, 0, openIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val startPending = servicePendingIntent(ACTION_START, 1)
        val stopPending = servicePendingIntent(ACTION_STOP, 2)

        val text = when {
            error != null -> error
            running -> "Core berjalan di ${CoreBridge.API_BASE}"
            else -> "Memulai core…"
        }

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("WebForge Mobile")
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentIntent(openPending)
            .setOngoing(running)
            .setOnlyAlertOnce(true)
            .addAction(0, "Start", startPending)
            .addAction(0, "Stop", stopPending)
            .build()
    }

    private fun servicePendingIntent(action: String, requestCode: Int): PendingIntent {
        val intent = Intent(this, WebForgeService::class.java).setAction(action)
        return PendingIntent.getService(
            this, requestCode, intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
    }

    companion object {
        const val ACTION_START = "com.webforge.mobile.action.START_CORE"
        const val ACTION_STOP = "com.webforge.mobile.action.STOP_CORE"
        private const val CHANNEL_ID = "webforge_core"
        private const val NOTIFICATION_ID = 1001

        private const val TAG = "WebForgeService"
        private const val WAKELOCK_TAG = "webforge:server"
        private const val WAKELOCK_TIMEOUT_MS = 10 * 60 * 1000L
        private const val WAKELOCK_CHECK_SECONDS = 15L
        private const val WATCHDOG_CHECK_SECONDS = 30L
        private const val HTTP_TIMEOUT_MS = 2000
        private const val MAX_RESTART_ATTEMPTS = 3
        private const val RESTART_DELAY_MS = 5000L

        private const val PREFS_NAME = "webforge_power"
        private const val KEY_KEEP_CPU_AWAKE = "keep_cpu_awake"

        /** Instance service yang sedang hidup, untuk evaluasi ulang segera. */
        @Volatile
        private var activeInstance: WebForgeService? = null

        /** Toggle keepCpuAwake dari SharedPreferences (default true). */
        fun readKeepCpuAwake(context: Context): Boolean =
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                .getBoolean(KEY_KEEP_CPU_AWAKE, true)

        fun writeKeepCpuAwake(context: Context, enabled: Boolean) {
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                .edit()
                .putBoolean(KEY_KEEP_CPU_AWAKE, enabled)
                .apply()
        }

        /**
         * Menerapkan perubahan toggle segera: service yang hidup menilai
         * ulang WakeLock sekarang juga, bukan menunggu siklus 15 detik.
         * Bila service tidak hidup, tidak ada WakeLock yang perlu diubah.
         */
        fun requestWakeLockReevaluation() {
            activeInstance?.evaluateWakeLockAsync()
        }

        fun start(context: Context) {
            val intent = Intent(context, WebForgeService::class.java).setAction(ACTION_START)
            ContextCompat.startForegroundService(context, intent)
        }

        fun stop(context: Context) {
            val intent = Intent(context, WebForgeService::class.java).setAction(ACTION_STOP)
            // Service mungkin sudah berjalan; startService cukup untuk
            // mengirim aksi STOP ke instance yang hidup.
            context.startService(intent)
        }
    }
}
