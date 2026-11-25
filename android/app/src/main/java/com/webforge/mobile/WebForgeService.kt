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
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat

/**
 * Foreground service yang menjaga Rust core tetap hidup selama pengguna
 * membutuhkannya. Service ini pemilik lifecycle core: start/stop core
 * selalu lewat sini (plugin dan notifikasi memicu aksi yang sama).
 *
 * Tipe FGS: specialUse (targetSdk 35 mewajibkan tipe eksplisit).
 * Subtype dinyatakan jujur di AndroidManifest: server pengembangan lokal
 * yang berjalan di perangkat atas permintaan pengguna.
 */
class WebForgeService : Service() {

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopCoreAndSelf()
                return START_NOT_STICKY
            }
            else -> {
                // ACTION_START atau restart oleh sistem (intent null dengan
                // START_STICKY): tampilkan notifikasi dulu (batas 5 detik),
                // lalu mulai core.
                startForegroundCompat(buildNotification(running = CoreBridge.started, error = null))
                Thread {
                    val ok = CoreBridge.start(applicationContext)
                    val error = when {
                        ok -> null
                        !CoreBridge.libraryLoaded ->
                            "Library native tidak termuat: ${CoreBridge.loadError ?: "tidak diketahui"}"
                        else -> "Core gagal dimulai (nativeStart mengembalikan error)"
                    }
                    updateNotification(buildNotification(running = ok, error = error))
                }.start()
                return START_STICKY
            }
        }
    }

    override fun onDestroy() {
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
