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

    private companion object {
        /** Same 10 MB cap as the Windows client. */
        const val MAX_BODY_BYTES = 10 * 1024 * 1024
    }

    private fun readBounded(input: java.io.InputStream, max: Int): String {
        val out = java.io.ByteArrayOutputStream()
        val buf = ByteArray(8192)
        input.use { s ->
            while (true) {
                val n = s.read(buf)
                if (n < 0) break
                if (out.size() + n > max) {
                    throw IllegalStateException("subscription exceeds $max bytes")
                }
                out.write(buf, 0, n)
            }
        }
        return out.toString(Charsets.UTF_8.name())
    }

    /** Fetch + normalize a subscription URL into [Node]s. */
    suspend fun fetch(url: String): List<Node> = withContext(Dispatchers.IO) {
        Logger.info("fetching subscription")
        val request = Request.Builder().url(url).build()
        client.newCall(request).execute().use { resp ->
            if (!resp.isSuccessful) {
                throw IllegalStateException("HTTP ${resp.code} for subscription")
            }
            // Bounded read: a hostile server must not be able to stream unbounded data into memory.
            val body = resp.body?.let { readBounded(it.byteStream(), MAX_BODY_BYTES) }.orEmpty()
            normalize(body)
        }
    }

    /**
     * Light normalizer for a Clash `proxies:` section (flow `- { name: a, type: ss, ... }`
     * and block `- name: a` styles). Keeps only name/type/server — still an MVP: the
     * credentials are NOT retained, so the generated config cannot connect yet (see README
     * "Status"). Stops at the next top-level key so `proxy-groups:` etc. are never read as nodes.
     */
    fun normalize(body: String): List<Node> {
        val nodes = mutableListOf<Node>()
        var inProxies = false
        var cur = mutableMapOf<String, String>()

        fun flush() {
            val name = cur["name"]
            val type = cur["type"]
            val server = cur["server"]
            if (name != null && type != null && server != null && nodes.none { it.name == name }) {
                nodes.add(Node(name = name, type = type, server = server))
            }
            cur = mutableMapOf()
        }

        for (raw in body.lineSequence()) {
            val line = raw.trimEnd()
            if (line.isBlank() || line.trimStart().startsWith("#")) continue
            val topLevel = !line.first().isWhitespace() && !line.startsWith("-")
            if (topLevel) {
                if (inProxies) flush()
                inProxies = line.startsWith("proxies:")
                continue
            }
            if (!inProxies) continue
            val t = line.trim()
            if (t.startsWith("-")) {
                flush()
                collectFields(t.removePrefix("-").trim(), cur)
            } else {
                collectFields(t, cur)
            }
        }
        if (inProxies) flush()
        return nodes
    }

    /** Parses `k: v` pairs from either a flow mapping (`{ a: 1, b: 2 }`) or a single block line. */
    private fun collectFields(fragment: String, into: MutableMap<String, String>) {
        val inner = fragment.trim().removePrefix("{").removeSuffix("}")
        for (key in listOf("name", "type", "server")) {
            valueOf(inner, key)?.let { into[key] = it }
        }
    }

    private fun valueOf(fragment: String, key: String): String? {
        val m = Regex("""(?:^|[\s{,])$key\s*[:=]\s*("[^"]*"|'[^']*'|[^,}]+)""").find(fragment) ?: return null
        val v = m.groupValues[1].trim()
        return when {
            v.length >= 2 && v.startsWith("\"") && v.endsWith("\"") -> v.substring(1, v.length - 1)
            v.length >= 2 && v.startsWith("'") && v.endsWith("'") -> v.substring(1, v.length - 1)
            else -> v
        }.ifEmpty { null }
    }
}
