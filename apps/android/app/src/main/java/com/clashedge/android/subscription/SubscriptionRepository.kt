package com.clashedge.android.subscription

import com.clashedge.android.config.AppConfigStore
import com.clashedge.android.model.Node
import com.clashedge.android.util.Logger
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request

/**
 * Downloads + normalizes a subscription URL into a nodes-only list.
 *
 * Platform boundary: "subscription provides nodes, the app owns the policy" — same
 * rule as Windows. Only `proxies`/`proxy-providers` are extracted; the subscription
 * never supplies proxy-groups/rules (the app always applies the built-in group
 * skeleton from RuntimeConfigGenerator).
 */
class SubscriptionRepository(private val appConfig: AppConfigStore) {

    private val client = OkHttpClient.Builder()
        .connectTimeout(30, TimeUnit.SECONDS)
        .readTimeout(30, TimeUnit.SECONDS)
        .build()

    /** Fetch + normalize a subscription URL into [Node]s. */
    suspend fun fetch(url: String): List<Node> = withContext(Dispatchers.IO) {
        Logger.info("fetching subscription")
        val request = Request.Builder().url(url).build()
        client.newCall(request).execute().use { resp ->
            if (!resp.isSuccessful) {
                throw IllegalStateException("HTTP ${resp.code} for subscription")
            }
            val body = resp.body?.string().orEmpty()
            normalize(body)
        }
    }

    /**
     * Light normalizer: pulls a Clash `proxies:` block and keeps only
     * name/type/server. MVP — expand full Clash parsing later.
     */
    fun normalize(body: String): List<Node> {
        val nodes = mutableListOf<Node>()
        val proxiesBlock = body.substringAfter("proxies:", "")
        proxiesBlock.lineSequence().forEach { line ->
            val t = line.trim()
            if (!t.startsWith("-")) return@forEach
            parseClashNode(t)?.let { if (nodes.none { n -> n.name == it.name }) nodes.add(it) }
        }
        return nodes
    }

    private fun parseClashNode(fragment: String): Node? {
        val name = keyOf(fragment, "name") ?: return null
        val type = keyOf(fragment, "type") ?: return null
        val server = keyOf(fragment, "server") ?: return null
        return Node(name = name, type = type, server = server)
    }

    private fun keyOf(fragment: String, key: String): String? {
        val idx = fragment.indexOf("$key=")
        if (idx < 0) return null
        var rest = fragment.substring(idx + key.length + 1).trim()
        if (rest.startsWith("\"")) {
            val end = rest.indexOf('"', 1)
            return if (end > 0) rest.substring(1, end) else null
        }
        if (rest.startsWith("'")) {
            val end = rest.indexOf('\'', 1)
            return if (end > 0) rest.substring(1, end) else null
        }
        rest = rest.substringBefore(',')
        return rest.trim().trimEnd('}')
    }
}
