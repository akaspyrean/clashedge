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
}
