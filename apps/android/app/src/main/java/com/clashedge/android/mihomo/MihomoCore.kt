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
 * JNI implementation that calls the Mihomo AAR (gomobile-generated bindings).
 * The gomobile-generated class is `mihomo.Mihomo` with static methods.
 */
class MihomoCoreImpl : MihomoCore {

    private val _running = kotlinx.coroutines.flow.MutableStateFlow(false)
    override val running: StateFlow<Boolean> = _running
    private val _error = kotlinx.coroutines.flow.MutableStateFlow<String?>(null)
    override val error: StateFlow<String?> = _error

    override fun start(configYaml: String, tunFd: Int): String {
        _error.value = null
        try {
            // Call the gomobile-generated JNI binding
            val result = mihomo.Mihomo.start(configYaml, tunFd)
            if (result.isEmpty()) {
                _running.value = true
                _error.value = null
                com.clashedge.android.util.Logger.info("Mihomo core started (config ${configYaml.length} bytes, tunFd=$tunFd)")
                return ""
            } else {
                _error.value = result
                return result
            }
        } catch (e: Exception) {
            val msg = "JNI start failed: ${e.message}"
            _error.value = msg
            com.clashedge.android.util.Logger.error("Mihomo core start failed: $e")
            return msg
        }
    }

    override fun stop() {
        try {
            val result = mihomo.Mihomo.stop()
            _running.value = false
            if (result.isNotBlank()) {
                _error.value = result
                com.clashedge.android.util.Logger.warn("Mihomo core stop returned: $result")
            } else {
                _error.value = null
            }
            com.clashedge.android.util.Logger.info("Mihomo core stopped")
        } catch (e: Exception) {
            val msg = "JNI stop failed: ${e.message}"
            _error.value = msg
            com.clashedge.android.util.Logger.error("Mihomo core stop failed: $e")
        }
    }

    override fun setMode(mode: String) {
        try {
            val result = mihomo.Mihomo.setMode(mode)
            if (result.isNotBlank()) {
                _error.value = result
                com.clashedge.android.util.Logger.warn("Mihomo setMode returned: $result")
            }
            com.clashedge.android.util.Logger.info("Mihomo proxy mode -> $mode")
        } catch (e: Exception) {
            val msg = "JNI setMode failed: ${e.message}"
            _error.value = msg
            com.clashedge.android.util.Logger.error("Mihomo setMode failed: $e")
        }
    }

    override fun selectProxy(group: String, node: String) {
        try {
            val result = mihomo.Mihomo.selectProxy(group, node)
            if (result.isNotBlank()) {
                _error.value = result
                com.clashedge.android.util.Logger.warn("Mihomo selectProxy returned: $result")
            }
            com.clashedge.android.util.Logger.info("select $group -> $node")
        } catch (e: Exception) {
            val msg = "JNI selectProxy failed: ${e.message}"
            _error.value = msg
            com.clashedge.android.util.Logger.error("Mihomo selectProxy failed: $e")
        }
    }

    override fun queryGroups(): List<ProxyGroup> {
        try {
            // The gomobile binding returns a JSON string or list; we'll parse it
            val json = mihomo.Mihomo.getProxies()
            // TODO: Parse JSON into List<ProxyGroup>
            // For now, return empty list until parsing is implemented
            return emptyList()
        } catch (e: Exception) {
            com.clashedge.android.util.Logger.error("Mihomo queryGroups failed: $e")
            return emptyList()
        }
    }
}