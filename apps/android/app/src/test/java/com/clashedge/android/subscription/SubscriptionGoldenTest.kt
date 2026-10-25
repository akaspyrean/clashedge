package com.clashedge.android.subscription

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.contentOrNull
import org.junit.Assert
import org.junit.Test
import java.io.File

/**
 * Golden fixtures test (D4-3): consumes the shared fixtures from repo root
 * `tests/fixtures/subscription/` and proves the documented parser gap:
 *
 * The expected JSON (produced by the Windows golden test) carries the full
 * credential field set (password / uuid / cipher / alterId / tls / sni / ...),
 * while Android's Node model only keeps name / type / server.
 *
 * This is an explicit RED LIGHT: the failure report below is the quantified
 * cross-platform drift. Fixing the Android parser belongs to the real kernel
 * integration milestone — NOT a quick patch here (D4-5 forbids that).
 */
class SubscriptionGoldenTest {

    /** Fields Android's Node model actually retains. */
    private val androidRetainedFields = setOf("name", "type", "server")

    /** Credential / transport fields the Windows parser produces per node type. */
    private val credentialFields = setOf(
        "password", "uuid", "cipher", "alterId", "tls", "servername", "sni",
        "network", "skip-cert-verify", "flow", "congestion-controller",
        "udp-relay-mode", "alpn", "plugin", "plugin-opts", "obfs",
        "obfs-password", "udp", "client-fingerprint", "reality-opts",
        "ws-opts", "grpc-opts", "h2-opts", "ports",
    )

    @Test
    fun goldenFixturesExposeAndroidParserGap() {
        // Repo root via system property injected by Gradle (testOptions.unitTests.all).
        // Falls back to the historical parentFile chain if missing.
        val repoRoot = File(
            System.getProperty("clashedge.repoRoot")
                ?: File(System.getProperty("user.dir")!!).parentFile!!.parentFile!!.parentFile!!.absolutePath,
        )
        val fixturesDir = File(repoRoot, "tests/fixtures/subscription")
        Assert.assertTrue(
            "shared fixtures dir not found at ${fixturesDir.absolutePath} — clone the repo fully or wire the Gradle sourceSet (D4-3)",
            fixturesDir.isDirectory,
        )

        val inputs = fixturesDir.listFiles()
            ?.filter { it.isFile && (it.extension == "yaml" || it.extension == "txt") }
            ?.sortedBy { it.name }
            .orEmpty()
        Assert.assertTrue("no fixture inputs found under ${fixturesDir.absolutePath}", inputs.isNotEmpty())

        val json = Json { ignoreUnknownKeys = true }
        val failures = mutableListOf<String>()

        for (input in inputs) {
            val expectedFile = File(fixturesDir, "${input.nameWithoutExtension}.expected.json")
            if (!expectedFile.isFile) {
                failures.add("${input.name}: missing ${expectedFile.name}")
                continue
            }
            val expectedArray = json.parseToJsonElement(expectedFile.readText()).jsonArray
            for ((idx, el) in expectedArray.withIndex()) {
                val node = el.jsonObject
                val name = node["name"]?.jsonPrimitive?.contentOrNull ?: "(unnamed #$idx)"
                val missing = node.keys.filter { it in credentialFields && it !in androidRetainedFields }
                if (missing.isNotEmpty()) {
                    failures.add(
                        "${input.name}: node '$name' — Android Node model DROPS credential fields: ${missing.sorted().joinToString(", ")}",
                    )
                }
            }
        }

        // RED LIGHT: every fixture with credentials is dropped by the Android
        // parser; the report turns the documented gap into a failing test.
        Assert.assertTrue(
            "Android parser credential gap (see report):\n" + failures.joinToString("\n"),
            failures.isEmpty(),
        )
    }

    /** Sanity: the fixture set actually contains credential-bearing nodes. */
    @Test
    fun fixturesContainCredentialBearingNodes() {
        val repoRoot = File(
            System.getProperty("clashedge.repoRoot")
                ?: File(System.getProperty("user.dir")!!).parentFile!!.parentFile!!.parentFile!!.absolutePath,
        )
        val fixturesDir = File(repoRoot, "tests/fixtures/subscription")
        if (!fixturesDir.isDirectory) return // CI without repo checkout: skip
        val expectedFiles = fixturesDir.listFiles { f -> f.extension == "json" }.orEmpty()
        Assert.assertTrue("no .expected.json files found", expectedFiles.isNotEmpty())
        val json = Json { ignoreUnknownKeys = true }
        val hasCredentials = expectedFiles.any { f ->
            json.parseToJsonElement(f.readText()).jsonArray.any { el ->
                el.jsonObject.keys.any { it in credentialFields }
            }
        }
        Assert.assertTrue("fixtures should contain at least one credential-bearing node", hasCredentials)
    }
}