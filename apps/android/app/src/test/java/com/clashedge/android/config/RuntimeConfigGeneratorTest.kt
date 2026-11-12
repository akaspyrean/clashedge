package com.clashedge.android.config

import com.clashedge.android.model.Node
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RuntimeConfigGeneratorTest {
    @Test fun wsOptionsUseNestedMihomoShape() {
        val node = Node(
            name = "node", type = "vmess", server = "example.com", port = 443,
            network = "ws", wsPath = "/ws", wsHeaders = mapOf("Host" to "edge.example.com")
        )
        val config = RuntimeConfigGenerator.build("rule", listOf(node), "/data/rules")
        assertTrue(config.contains("ws-opts: { path: \"/ws\", headers: { \"Host\": \"edge.example.com\" } }"))
        assertFalse(config.contains("ws-headers:"))
    }

    @Test fun quotedNodeValuesCannotInjectNewYamlLines() {
        val node = Node(name = "bad\nmode: direct", type = "ss", server = "example.com", port = 443)
        val config = RuntimeConfigGenerator.build("rule", listOf(node), "/data/rules")
        assertTrue(config.contains("bad\\nmode: direct"))
        assertFalse(config.contains("bad\nmode: direct"))
    }

    /**
     * P0 稳定性（与 Windows 端同步）：
     * - P0-1 选择持久化：profile.store-selected / store-fake-ip 必须写入，
     *   配置重载后手动选择由 mihomo cache.db 恢复；
     * - P0-2 探测盲区收敛：interval 300→120 + max-failed-times=2。
     */
    @Test fun persistsSelectionAndShrinksProbeBlindWindow() {
        val node = Node(name = "n1", type = "ss", server = "example.com", port = 443)
        val config = RuntimeConfigGenerator.build("rule", listOf(node), "/data/rules")
        assertTrue(config.contains("store-selected: true"))
        assertTrue(config.contains("store-fake-ip: true"))
        assertTrue(config.contains("interval: 120"))
        assertTrue(config.contains("max-failed-times: 2"))
        assertFalse(config.contains("interval: 300"))
    }
}
