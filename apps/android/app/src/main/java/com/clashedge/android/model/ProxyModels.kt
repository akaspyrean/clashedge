package com.clashedge.android.model

import kotlinx.serialization.Serializable

/** Real runtime state of the proxy. The UI renders exactly this. */
enum class ConnectionState {
    STOPPED,
    STARTING,
    RUNNING,
    STOPPING,
    ERROR,
}

/**
 * Basic connection info surfaced by Mihomo. Kept deliberately small for the MVP
 * (rule selectors add traffic/connection detail in a later milestone).
 * Credentials are retained so generated configs can actually connect.
 */
@Serializable
data class Node(
    val name: String,
    val type: String,
    val server: String,
    val port: Int = 0,
    // Common credentials
    val password: String? = null,
    val uuid: String? = null,
    val cipher: String? = null,
    val udp: Boolean = false,
    // TLS/TLS-related
    val tls: Boolean = false,
    val sni: String? = null,
    val alpn: List<String> = emptyList(),
    val fingerprint: String? = null,
    // WS/HTTP/QUIC
    val network: String = "tcp",
    val wsPath: String? = null,
    val wsHeaders: Map<String, String> = emptyMap(),
    val httpPath: String? = null,
    val httpHeaders: Map<String, String> = emptyMap(),
    // Hysteria2
    val upMbps: Int? = null,
    val downMbps: Int? = null,
    val authStr: String? = null,
    // Shadowsocks
    val method: String? = null,
    // VLESS
    val flow: String? = null,
    // VMESS
    val alterId: Int? = null,
    val security: String? = null,
)

@Serializable
data class ProxyGroup(
    val name: String,
    val type: String,
    val nodes: List<String>,
    val now: String? = null,
)

/** A subscription-derived provider list (only nodes are imported, like Windows). */
@Serializable
data class Profile(
    val id: String,
    val name: String,
    val subscribeUrl: String? = null,
    val content: String,
)

@Serializable
data class LogEntry(
    val time: Long,
    val level: String,
    val message: String,
)
