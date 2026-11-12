package com.clashedge.android.config

import com.clashedge.android.model.Node

/**
 * Builds the Mihomo runtime config from the active profile's nodes + the app mode.
 *
 * Shares the Windows proxy-group naming and default rule chain (same product
 * rules / same group naming) so a ClashEdge user keeps a familiar information
 * architecture across platforms. Rule-set files are pinned in `assets.lock.json` (external repo) and are
 * copied into the data dir at first launch.
 */
object RuntimeConfigGenerator {

    const val GROUP_LADDER = "扶梯出行"
    const val GROUP_AI = "人工智能"
    const val GROUP_MEDIA = "影音视听"
    const val GROUP_MANUAL = "人工优选"
    const val GROUP_AUTO = "自动优选"

    private val VALID_MODES = setOf("rule", "global", "direct")

    fun build(mode: String, nodes: List<Node>, rulesDir: String): String {
        // mihomo only accepts rule / global / direct; anything else would corrupt the YAML.
        val safeMode = if (mode in VALID_MODES) mode else "rule"
        val proxyLines = nodes.map { n ->
            buildString {
                append("  - { name: ")
                append(escapeYaml(n.name))
                append(", type: ")
                append(escapeYaml(n.type))
                append(", server: ")
                append(escapeYaml(n.server))
                if (n.port > 0) { append(", port: "); append(n.port.toString()) }
                n.password?.let { append(", password: "); append(escapeYaml(it)) }
                n.uuid?.let { append(", uuid: "); append(escapeYaml(it)) }
                n.cipher?.let { append(", cipher: "); append(escapeYaml(it)) }
                if (n.udp) append(", udp: true")
                if (n.tls) append(", tls: true")
                n.sni?.let { append(", sni: "); append(escapeYaml(it)) }
                if (n.alpn.isNotEmpty()) { append(", alpn: ["); append(n.alpn.map { escapeYaml(it) }.joinToString(", ")); append("]") }
                n.fingerprint?.let { append(", fingerprint: "); append(escapeYaml(it)) }
                if (n.network != "tcp") { append(", network: "); append(escapeYaml(n.network)) }
                if (n.wsPath != null || n.wsHeaders.isNotEmpty()) {
                    append(", ws-opts: { ")
                    val parts = mutableListOf<String>()
                    n.wsPath?.let { parts += "path: ${escapeYaml(it)}" }
                    if (n.wsHeaders.isNotEmpty()) {
                        parts += "headers: { " + n.wsHeaders.entries.joinToString(", ") { (k, v) ->
                            "${escapeYaml(k)}: ${escapeYaml(v)}"
                        } + " }"
                    }
                    append(parts.joinToString(", "))
                    append(" }")
                }
                if (n.httpPath != null || n.httpHeaders.isNotEmpty()) {
                    append(", http-opts: { ")
                    val parts = mutableListOf<String>()
                    n.httpPath?.let { parts += "path: [${escapeYaml(it)}]" }
                    if (n.httpHeaders.isNotEmpty()) {
                        parts += "headers: { " + n.httpHeaders.entries.joinToString(", ") { (k, v) ->
                            "${escapeYaml(k)}: [${escapeYaml(v)}]"
                        } + " }"
                    }
                    append(parts.joinToString(", "))
                    append(" }")
                }
                n.upMbps?.let { append(", up: "); append(it.toString()) }
                n.downMbps?.let { append(", down: "); append(it.toString()) }
                n.authStr?.let { append(", auth-str: "); append(escapeYaml(it)) }
                n.method?.let { append(", method: "); append(escapeYaml(it)) }
                n.flow?.let { append(", flow: "); append(escapeYaml(it)) }
                n.alterId?.let { append(", alter-id: "); append(it.toString()) }
                n.security?.let { append(", security: "); append(escapeYaml(it)) }
                append(" }")
            }
        }.joinToString("\n")

        val manualMembers = nodes.map { escapeYaml(it.name) }
        // url-test must contain REAL nodes only: DIRECT is not a proxy, url-test would pick it as
        // the "fastest" node forever (same rule as the Windows build_runtime_config). With no
        // nodes mihomo rejects an empty group, so the auto group is dropped and the manual
        // group falls back to DIRECT.
        val hasNodes = manualMembers.isNotEmpty()
        val manualList = if (hasNodes) manualMembers.joinToString(", ") else "DIRECT"
        val leafRefs = if (hasNodes) "$GROUP_MANUAL, $GROUP_AUTO" else GROUP_MANUAL

        return buildString {
            appendLine("mixed-port: 7890")
            appendLine("allow-lan: false")
            appendLine("mode: $safeMode")
            appendLine("log-level: info")
            appendLine()
            // P0-1 选择持久化：与 Windows 端 build_runtime_config 同步。store-selected
            // 让 mihomo 把手动选择写入 cache.db，配置重载后按名称恢复；
            // store-fake-ip 持久化 fake-ip 映射，避免重载后解析抖动。
            appendLine("profile:")
            appendLine("  store-selected: true")
            appendLine("  store-fake-ip: true")
            appendLine()
            appendLine("dns:")
            appendLine("  enable: true")
            appendLine("  listen: 127.0.0.1:1053")
            appendLine("  enhanced-mode: fake-ip")
            appendLine()
            appendLine("tun:")
            appendLine("  enable: true")
            appendLine("  stack: gvisor")
            appendLine("  auto-route: false")
            appendLine("  auto-detect-interface: false")
            appendLine()
            appendLine("proxies:")
            if (proxyLines.isBlank()) appendLine("  []") else appendLine(proxyLines)
            appendLine()
            appendLine("proxy-groups:")
            appendLine("  - name: GLOBAL")
            appendLine("    type: select")
            appendLine("    proxies: [DIRECT, REJECT, $leafRefs]")
            appendLine("  - name: $GROUP_LADDER")
            appendLine("    type: select")
            appendLine("    proxies: [$leafRefs]")
            appendLine("  - name: $GROUP_AI")
            appendLine("    type: select")
            appendLine("    proxies: [$leafRefs]")
            appendLine("  - name: $GROUP_MEDIA")
            appendLine("    type: select")
            appendLine("    proxies: [$leafRefs]")
            appendLine("  - name: $GROUP_MANUAL")
            appendLine("    type: select")
            appendLine("    proxies: [$manualList]")
            if (hasNodes) {
                appendLine("  - name: $GROUP_AUTO")
                appendLine("    type: url-test")
                appendLine("    url: https://cp.cloudflare.com/generate_204")
                // P0-2 探测盲区收敛（与 Windows 端 default_proxy_groups 同步）：
                // interval 300→120；max-failed-times=2 连败即刻强制检查。
                appendLine("    interval: 120")
                appendLine("    max-failed-times: 2")
                appendLine("    tolerance: 100")
                appendLine("    proxies: [${manualMembers.joinToString(", ")}]")
            }
            appendLine()
            appendRuleProviders(rulesDir)
            appendLine("rules:")
            appendLine("  - GEOSITE,private,DIRECT")
            appendLine("  - RULE-SET,direct,DIRECT")
            appendLine("  - RULE-SET,ad,REJECT")
            appendLine("  - GEOSITE,category-ads-all,REJECT")
            appendLine("  - RULE-SET,ai,$GROUP_AI")
            appendLine("  - RULE-SET,media,$GROUP_MEDIA")
            appendLine("  - RULE-SET,proxy,$GROUP_LADDER")
            appendLine("  - GEOSITE,cn,DIRECT")
            appendLine("  - GEOIP,CN,DIRECT")
            appendLine("  - MATCH,$GROUP_LADDER")
        }
    }

    private fun StringBuilder.appendRuleProviders(rulesDir: String) {
        appendLine("rule-providers:")
        listOf(
            "direct" to "direct.yaml",
            "proxy" to "proxy.yaml",
            "media" to "media.yaml",
            "ai" to "ai.yaml",
            "ad" to "ad.yaml",
        ).forEach { (name, file) ->
            appendLine("  $name:")
            appendLine("    type: file")
            appendLine("    behavior: classical")
            appendLine("    path: $rulesDir/$file")
        }
    }

    private fun escapeYaml(value: String): String = buildString {
        append('"')
        for (char in value) {
            when (char) {
                '\\' -> append("\\\\")
                '"' -> append("\\\"")
                '\n' -> append("\\n")
                '\r' -> append("\\r")
                '\t' -> append("\\t")
                else -> if (char.code < 0x20) append("\\u%04x".format(char.code)) else append(char)
            }
        }
        append('"')
    }
}
