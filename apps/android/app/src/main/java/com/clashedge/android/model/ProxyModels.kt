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
 */
@Serializable
data class Node(
    val name: String,
    val type: String,
    val server: String,
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
