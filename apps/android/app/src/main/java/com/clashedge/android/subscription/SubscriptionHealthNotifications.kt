package com.clashedge.android.subscription

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.clashedge.android.R

/** Best-effort subscription expiry and low-traffic notices. */
internal object SubscriptionHealthNotifications {
    private const val CHANNEL_ID = "subscription_health"
    private const val PREFS = "subscription_health_notifications"
    private const val WARNING_WINDOW_SECONDS = 7 * 24 * 60 * 60L

    fun notifyIfNeeded(context: Context, subscriptionKey: String, rawUserInfo: String?) {
        if (rawUserInfo.isNullOrBlank()) return
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(
                context, Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED
        ) return

        val fields = rawUserInfo.split(';').mapNotNull { part ->
            val pair = part.trim().split('=', limit = 2)
            if (pair.size == 2) pair[0].trim() to pair[1].trim().toLongOrNull() else null
        }.toMap()
        val now = System.currentTimeMillis() / 1000
        fields["expire"]?.let { expiry ->
            if (expiry >= now && expiry <= now + WARNING_WINDOW_SECONDS) {
                notifyOnce(context, "expiry:$subscriptionKey:$expiry", "Subscription expiring soon",
                    context.getString(R.string.subscription_expiry_notice))
            }
        }
        val total = fields["total"] ?: return
        val upload = fields["upload"]?.coerceAtLeast(0) ?: return
        val download = fields["download"]?.coerceAtLeast(0) ?: return
        val used = if (upload > Long.MAX_VALUE - download) Long.MAX_VALUE else upload + download
        if (total > 0 && (total - used).coerceAtLeast(0) <= total / 10) {
            notifyOnce(context, "traffic:$subscriptionKey:$total", "Subscription traffic low",
                context.getString(R.string.subscription_traffic_notice))
        }
    }

    private fun notifyOnce(context: Context, key: String, title: String, message: String) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (prefs.getBoolean(key, false)) return
        val manager = NotificationManagerCompat.from(context)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(CHANNEL_ID, "Subscription status", NotificationManager.IMPORTANCE_DEFAULT)
            (context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager)
                .createNotificationChannel(channel)
        }
        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(title)
            .setContentText(message)
            .setAutoCancel(true)
            .build()
        try {
            manager.notify(key.hashCode() and 0x7fffffff, notification)
            prefs.edit().putBoolean(key, true).apply()
        } catch (_: SecurityException) {
            // Notification permission can be revoked between the permission check and notify().
        }
    }
}
