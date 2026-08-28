package com.clashedge.android.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import com.clashedge.android.MainActivity
import com.clashedge.android.R
import com.clashedge.android.vpn.ClashVpnService

/**
 * Foreground service that keeps the proxy process alive and delegates TUN setup to
 * ClashVpnService. Running the VPN as a foreground service prevents the system
 * from killing it under memory pressure — but if it IS killed anyway, the
 * coordinator flips state to STOPPED (never a stale "connected").
 */
class ProxyForegroundService : Service() {

    companion object {
        const val EXTRA_CONFIG = "config"
        const val EXTRA_MODE = "mode"
        const val CHANNEL_ID = "clashedge_proxy"
        const val NOTIFICATION_ID = 1
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(NOTIFICATION_ID, buildNotification())
        val vpn = Intent(this, ClashVpnService::class.java)
        vpn.putExtra(EXTRA_CONFIG, intent?.getStringExtra(EXTRA_CONFIG).orEmpty())
        vpn.putExtra(EXTRA_MODE, intent?.getStringExtra(EXTRA_MODE).orEmpty())
        startService(vpn)
        return START_STICKY
    }

    private fun buildNotification(): Notification {
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.nav_home),
            NotificationManager.IMPORTANCE_LOW,
        )
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager)
            .createNotificationChannel(channel)

        val pi = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(getString(R.string.status_connected))
            .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setContentIntent(pi)
            .setOngoing(true)
            .build()
    }
}
