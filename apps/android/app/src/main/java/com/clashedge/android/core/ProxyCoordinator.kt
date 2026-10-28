package com.clashedge.android.core

import android.content.Context
import android.content.Intent
import android.net.VpnService
import com.clashedge.android.config.AppConfigStore
import com.clashedge.android.config.RuntimeConfigGenerator

import com.clashedge.android.mihomo.MihomoCore
import com.clashedge.android.mihomo.MihomoCoreImpl
import com.clashedge.android.model.ConnectionState
import com.clashedge.android.model.ProxyGroup
import com.clashedge.android.repository.ProfileRepository
import com.clashedge.android.service.ProxyForegroundService
import com.clashedge.android.util.Logger
import com.clashedge.android.vpn.ClashVpnService
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

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

    /** Last error message that caused ERROR state; retained until next successful start. */
    private val _lastError = MutableStateFlow<String?>(null)
    val lastError: StateFlow<String?> = _lastError.asStateFlow()

    /** Mutex to serialize start/stop and prevent race conditions. */
    private val lifecycleMutex = Mutex()

    init {
        // Observe MihomoCore.running/error and keep local state in sync (single source of truth).
        scope.launch {
            core.running.collect { isRunning ->
                if (isRunning && _state.value != ConnectionState.RUNNING) {
                    _state.value = ConnectionState.RUNNING
                    _error.value = null
                    _lastError.value = null
                    refreshGroups()
                } else if (!isRunning && _state.value == ConnectionState.RUNNING) {
                    _state.value = ConnectionState.STOPPED
                }
            }
        }
        scope.launch {
            core.error.collect { err ->
                if (err != null) {
                    _error.value = err
                    _lastError.value = err
                    if (_state.value == ConnectionState.RUNNING) {
                        _state.value = ConnectionState.ERROR
                    }
                }
            }
        }
    }

    /** Returns an activity result contract's launcher input if VPN permission needed. */
    fun vpnPermissionIntent(): Intent? {
        val p = VpnService.prepare(context)
        return if (p == null) null else p
    }

    suspend fun start() = lifecycleMutex.withLock { startLocked() }

    private suspend fun startLocked() {
        if (_state.value == ConnectionState.RUNNING) return
        if (_state.value == ConnectionState.STARTING || _state.value == ConnectionState.STOPPING) {
            Logger.warn("start() called during transition; ignoring")
            return
        }
        _state.value = ConnectionState.STARTING
        _error.value = null
        try {
            val settings = appConfig.settings.first()
            val rulesDir = profiles.rulesDir()
            val config = RuntimeConfigGenerator.build(
                mode = settings.mode,
                nodes = profiles.activeNodes(),
                rulesDir = rulesDir,
            )
            val svc = Intent(context, ProxyForegroundService::class.java)
                .putExtra(ProxyForegroundService.EXTRA_CONFIG, config)
                .putExtra(ProxyForegroundService.EXTRA_MODE, settings.mode)
            context.startForegroundService(svc)
        } catch (t: Throwable) {
            _error.value = t.message
            _lastError.value = t.message
            _state.value = ConnectionState.ERROR
            Logger.error("proxy start failed: $t")
        }
    }

    private suspend fun stopLocked() {
        _state.value = ConnectionState.STOPPING
        core.stop()
        context.stopService(Intent(context, ProxyForegroundService::class.java))
        context.stopService(Intent(context, ClashVpnService::class.java))
        // Keep _error if it was ERROR, only clear on explicit stop
        if (_state.value != ConnectionState.ERROR) {
            _error.value = null
        }
        _state.value = ConnectionState.STOPPED
    }

    fun stop() = scope.launch { lifecycleMutex.withLock { stopLocked() } }

    suspend fun restartCore() = lifecycleMutex.withLock {
        when (_state.value) {
            ConnectionState.RUNNING, ConnectionState.ERROR -> stopLocked()
            else -> {}
        }
        startLocked()
    }

    suspend fun reloadCore() = lifecycleMutex.withLock {
        _groups.value = core.queryGroups()
    }

    fun onCoreRunning() {
        _state.value = ConnectionState.RUNNING
        // Clear error on successful start
        _error.value = null
        _lastError.value = null
        refreshGroups()
    }

    /** Called by ClashVpnService once the TUN fd is ready. */
    fun onVpnEstablished(fd: Int, config: String) {
        scope.launch {
            try {
                val err = core.start(config, fd)
                if (err.isEmpty()) onCoreRunning()
                else {
                    onCoreError("Mihomo core failed to start: $err")
                    stop()
                }
            } catch (t: Throwable) {
                onCoreError("proxy start failed: ${t.message}")
                stop()
            }
        }
    }

    fun onCoreStopped(reason: String?) {
        if (reason != null) {
            _error.value = reason
            _lastError.value = reason
        }
        _state.value = ConnectionState.STOPPED
        refreshGroups()
    }

    fun onCoreError(message: String) {
        _error.value = message
        _lastError.value = message
        _state.value = ConnectionState.ERROR
        stop()
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
