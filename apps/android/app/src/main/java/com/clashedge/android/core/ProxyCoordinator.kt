package com.clashedge.android.core

import android.content.Context
import android.content.Intent
import android.net.VpnService
import com.clashedge.android.config.AppConfigStore
import com.clashedge.android.config.RuntimeConfigGenerator
import com.clashedge.android.mihomo.Box
import com.clashedge.android.mihomo.MihomoCore
import com.clashedge.android.mihomo.MihomoCoreImpl
import com.clashedge.android.model.ConnectionState
import com.clashedge.android.model.ProxyGroup
import com.clashedge.android.repository.ProfileRepository
import com.clashedge.android.service.ProxyForegroundService
import com.clashedge.android.util.Logger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Single source of truth for the proxy lifecycle. UI state must always reflect the
 * REAL core state, so this coordinator is the only thing that flips
 * [ConnectionState] between STOPPED and RUNNING.
 *
 * Error handling contract (section 12): any abnormal exit / killed service /
 * config damage flips state to ERROR or STOPPED — never leaves the UI stuck on a
 * fake "connected".
 */
class ProxyCoordinator(
    private val context: Context,
    private val appConfig: AppConfigStore,
    private val profiles: ProfileRepository,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val core: MihomoCore = MihomoCoreImpl()

    private val _state = MutableStateFlow(ConnectionState.STOPPED)
    val state: StateFlow<ConnectionState> = _state.asStateFlow()

    private val _groups = MutableStateFlow<List<ProxyGroup>>(emptyList())
    val groups: StateFlow<List<ProxyGroup>> = _groups.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    /** Returns an activity result contract's launcher input if VPN permission needed. */
    fun vpnPermissionIntent(): Intent? {
        val p = VpnService.prepare(context)
        return if (p == null) null else p
    }

    suspend fun start() {
        if (_state.value == ConnectionState.RUNNING) return
        _state.value = ConnectionState.STARTING
        _error.value = null
        try {
            val settings = appConfig.settings.first()
            val config = RuntimeConfigGenerator.build(
                mode = settings.mode,
                nodes = profiles.activeNodes(),
                rulesDir = profiles.rulesDir(),
            )
            // Start the TUN-owning foreground + VpnService; it hands the fd to the core.
            val svc = Intent(context, ProxyForegroundService::class.java)
                .putExtra(ProxyForegroundService.EXTRA_CONFIG, config)
                .putExtra(ProxyForegroundService.EXTRA_MODE, settings.mode)
            context.startForegroundService(svc)
            // Mihomo start is performed by the service once the TUN fd is ready.
        } catch (t: Throwable) {
            _error.value = t.message
            _state.value = ConnectionState.ERROR
            Logger.error("proxy start failed: $t")
        }
    }

    fun stop() {
        scope.launch {
            _state.value = ConnectionState.STOPPING
            core.stop()
            context.stopService(Intent(context, ProxyForegroundService::class.java))
            _state.value = ConnectionState.STOPPED
        }
    }

    fun onCoreRunning() {
        _state.value = ConnectionState.RUNNING
        refreshGroups()
    }

    /** Called by ClashVpnService once the TUN fd is ready. */
    fun onVpnEstablished(fd: Int, box: Box, config: String) {
        scope.launch {
            try {
                core.startTun(fd, box)
                if (core.start(config)) onCoreRunning()
                else onCoreError("Mihomo core failed to start")
            } catch (t: Throwable) {
                onCoreError("proxy start failed: ${t.message}")
            }
        }
    }

    fun onCoreStopped(reason: String?) {
        if (reason != null) _error.value = reason
        _state.value = ConnectionState.STOPPED
        refreshGroups()
    }

    fun onCoreError(message: String) {
        _error.value = message
        _state.value = ConnectionState.ERROR
    }

    fun refreshGroups() {
        scope.launch {
            _groups.value = core.queryGroups()
        }
    }

    fun setMode(mode: String) {
        scope.launch {
            appConfig.setMode(mode)
            core.setMode(mode)
            Logger.info("mode -> $mode")
        }
    }

    fun selectProxy(group: String, node: String) {
        core.selectProxy(group, node)
    }
}
