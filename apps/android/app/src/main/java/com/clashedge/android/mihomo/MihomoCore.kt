package com.clashedge.android.mihomo

import com.clashedge.android.model.ProxyGroup
import kotlinx.coroutines.flow.StateFlow

/**
 * Adapter around the Mihomo Android core (AAR / JNI).
 *
 * Platform boundary: the proxy core owns rules, DNS, proxy-groups and nodes, and
 * reads/writes the TUN file descriptor handed over by the Android VpnService. The
 * Windows implementation is a totally separate Tauri/Rust system layer and is NOT
 * reused here (see docs & README "ClashEdge for Android").
 *
 * The MVP wires TUN via [startTun] (fd from VpnService) and calls [start]; concrete
 * JNI binding is enabled when the Mihomo AAR artifact is pinned in
 * gradle/libs.versions.toml.
 */
interface MihomoCore {

    val running: StateFlow<Boolean>
    val error: StateFlow<String?>

    // Start with full YAML config and TUN fd (0 = no TUN)
    fun start(configYaml: String, tunFd: Int): String

    fun stop()

    fun setMode(mode: String)

    fun selectProxy(group: String, node: String)

    fun queryGroups(): List<ProxyGroup>
}

/**
 * Default in-memory implementation: real JNI calls must be wired to the Mihomo
 * AAR. Kept as an explicit boundary so the Android project compiles before the
 * artifact is pinned (the no-op keeps functions/logic coordinated by
 * ProxyCoordinator intact).
 */
class MihomoCoreImpl : MihomoCore {

    private val _running = kotlinx.coroutines.flow.MutableStateFlow(false)
    override val running: StateFlow<Boolean> = _running
    private val _error = kotlinx.coroutines.flow.MutableStateFlow<String?>(null)
    override val error: StateFlow<String?> = _error

    override fun start(configYaml: String, tunFd: Int): String {
        // TODO(aar): call MihomoCore.start(configYaml, tunFd) via JNI
        _error.value = null
        _running.value = true
        com.clashedge.android.util.Logger.info("Mihomo core start requested (config ${configYaml.length} bytes, tunFd=$tunFd)")
        return "" // empty = success
    }

    override fun stop() {
        _running.value = false
        com.clashedge.android.util.Logger.info("Mihomo core stop requested")
    }

    override fun setMode(mode: String) {
        com.clashedge.android.util.Logger.info("Mihomo proxy mode -> $mode")
    }

    override fun selectProxy(group: String, node: String) {
        com.clashedge.android.util.Logger.info("select $group -> $node")
    }

    override fun queryGroups(): List<ProxyGroup> = emptyList()
}