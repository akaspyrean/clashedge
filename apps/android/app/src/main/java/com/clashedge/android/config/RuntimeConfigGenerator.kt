package com.clashedge.android.config

import com.clashedge.android.model.ProxyGroup
import com.clashedge.android.model.Node

/**
 * Builds the Mihomo runtime config from the active profile's nodes + the app mode.
 *
 * Shares the Windows proxy-group naming and default rule chain (same product
 * rules / same group naming) so a ClashEdge user keeps a familiar information
 * architecture across platforms. Rule-set files come from `shared/rules` and are
 * copied into the data dir at first launch.
 */
object RuntimeConfigGenerator {

    const val GROUP_LADDER = "扶梯出行"
    const val GROUP_AI = "人工智能"
    const val GROUP_MEDIA = "影音视听"
    const val GROUP_MANUAL = "人工优选"
    const val GROUP_AUTO = "自动优选"

    fun build(mode: String, nodes: List<Node>, rulesDir: String): String {
        val proxyLines = nodes.map { n ->
            buildString {
                append("  - { name: ")
                append(escapeYaml(n.name))
                append(", type: ")
                append(n.type)
                append(", server: ")
                append(escapeYaml(n.server))
                append(" }")
            }
        }.joinToString("\n")

        val manualMembers = nodes.map { it.name }
        val autoMembers = manualMembers + "DIRECT"

        return buildString {
            appendLine("mixed-port: 7890")
            appendLine("allow-lan: false")
            appendLine("mode: $mode")
            appendLine("log-level: info")
            appendLine()
            appendLine("dns:")
            appendLine("  enable: true")
            appendLine("  listen: 0.0.0.0:1053")
            appendLine("  enhanced-mode: fake-ip")
            appendLine()
            appendLine("proxies:")
            if (proxyLines.isBlank()) appendLine("  []") else appendLine(proxyLines)
            appendLine()
            appendLine("proxy-groups:")
            appendLine("  - name: GLOBAL")
            appendLine("    type: select")
            appendLine("    proxies: [DIRECT, REJECT, $GROUP_MANUAL, $GROUP_AUTO]")
            appendLine("  - name: $GROUP_LADDER")
            appendLine("    type: select")
            appendLine("    proxies: [$GROUP_MANUAL, $GROUP_AUTO]")
            appendLine("  - name: $GROUP_AI")
            appendLine("    type: select")
            appendLine("    proxies: [$GROUP_MANUAL, $GROUP_AUTO]")
            appendLine("  - name: $GROUP_MEDIA")
            appendLine("    type: select")
            appendLine("    proxies: [$GROUP_MANUAL, $GROUP_AUTO]")
            appendLine("  - name: $GROUP_MANUAL")
            appendLine("    type: select")
            appendLine("    proxies: [${manualMembers.joinToString(", ")}]")
            appendLine("  - name: $GROUP_AUTO")
            appendLine("    type: url-test")
            appendLine("    url: https://cp.cloudflare.com/generate_204")
            appendLine("    interval: 300")
            appendLine("    tolerance: 100")
            appendLine("    proxies: [${autoMembers.joinToString(", ")}]")
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

    private fun escapeYaml(value: String): String =
        "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\""
}
