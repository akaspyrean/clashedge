package com.clashedge.android.vpn

import android.app.Service
import android.content.Intent
import android.net.VpnService
import android.os.IBinder
import com.clashedge.android.ClashEdgeApp
import com.clashedge.android.core.ProxyCoordinator
import com.clashedge.android.mihomo.Box
import com.clashedge.android.service.ProxyForegroundService
import com.clashedge.android.util.Logger

/**
 * Android VpnService: creates the TUN interface, exposes the resulting fd to
 * Mihomo, and tears everything down on revoke. All routing/DNS/rule work is done
 * by Mihomo, NOT here.
 *
 * State contract: if the VPN is revoked or dies while "connected", the UI must be
 * pushed to STOPPED/ERROR — [ProxyCoordinator] is the only writer of state.
 */
class ClashVpnService : VpnService() {

    private var started = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val config = intent?.getStringExtra(ProxyForegroundService.EXTRA_CONFIG)
            ?: return Service.START_NOT_STICKY
        if (started) return Service.START_STICKY

        val coordinator = coordinator()
        val fd = establish() ?: run {
            Logger.error("VPN establishment failed (permission not granted?)")
            coordinator.onCoreError("VPN establishment failed")
            stopSelf()
            return Service.START_NOT_STICKY
        }

        started = true
        return try {
            coordinator.onVpnEstablished(fd, tundata(), config)
            Service.START_STICKY
        } catch (t: Throwable) {
            Logger.error("vpn onStart error: $t")
            coordinator.onCoreError(t.toString())
            Service.START_NOT_STICKY
        }
    }

    override fun onRevoke() {
        Logger.warn("VPN revoked by the system")
        coordinator().onCoreStopped("VPN was revoked")
        started = false
        stopSelf()
        super.onRevoke()
    }

    private fun establish(): Int? {
        val builder = Builder()
            .setSession("ClashEdge")
            .setMtu(1400)
            .addAddress("172.19.0.1", 24)
            .addRoute("0.0.0.0", 0)
            .addDnsServer("223.5.5.5")
            .addDnsServer("119.29.29.29")
        val iface = builder.establish() ?: return null
        // keep the interface open; the fd is what Mihomo reads from/writes to.
        fileDescriptor = iface
        return iface.fd
    }

    private fun tundata(): Box = Box(
        mtu = 1400,
        addresses = listOf("172.19.0.1/24"),
        routes = listOf("0.0.0.0/0"),
        dnsServers = listOf("223.5.5.5", "119.29.29.29"),
    )

    private var fileDescriptor: android.os.ParcelFileDescriptor? = null

    private fun coordinator(): ProxyCoordinator =
        (application as ClashEdgeApp).coordinator
}
