package com.webforge.mobile

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.getcapacitor.BridgeActivity

class MainActivity : BridgeActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        // Plugin lokal harus terdaftar sebelum bridge dimuat.
        registerPlugin(WebForgePlugin::class.java)
        super.onCreate(savedInstanceState)

        requestNotificationPermissionIfNeeded()

        // Core dijalankan lewat foreground service agar tetap hidup saat
        // aplikasi di latar; frontend mengambil token/base dari plugin.
        WebForgeService.start(this)
    }

    private fun requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val granted = ContextCompat.checkSelfPermission(
            this, Manifest.permission.POST_NOTIFICATIONS
        ) == PackageManager.PERMISSION_GRANTED
        if (!granted) {
            ActivityCompat.requestPermissions(
                this,
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                REQUEST_NOTIFICATIONS
            )
        }
    }

    companion object {
        private const val REQUEST_NOTIFICATIONS = 2001
    }
}
