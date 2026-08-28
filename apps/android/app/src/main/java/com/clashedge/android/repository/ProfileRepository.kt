package com.clashedge.android.repository

import android.content.Context
import com.clashedge.android.config.AppConfigStore
import com.clashedge.android.model.Node
import com.clashedge.android.model.Profile
import com.clashedge.android.subscription.SubscriptionRepository
import com.clashedge.android.util.Logger
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

@Serializable
private data class ProfileRecord(
    val id: String,
    val name: String,
    val subscribeUrl: String? = null,
    val nodesJson: String,
)

/**
 * Profile storage + subscription import/refresh. The active profile determines the
 * node set fed to RuntimeConfigGenerator.
 */
class ProfileRepository(
    private val context: Context,
    private val subscriptions: SubscriptionRepository,
    private val appConfig: AppConfigStore,
) {
    private val json = Json { ignoreUnknownKeys = true }
    private val storeFile get() = File(context.filesDir, "profiles.json")

    private val _profiles = MutableStateFlow<List<Profile>>(emptyList())
    val profiles: StateFlow<List<Profile>> = _profiles.asStateFlow()

    init {
        load()
    }

    suspend fun activeProfile(): Profile? {
        val settings = appConfig.settings.first()
        return _profiles.value.firstOrNull { it.id == settings.activeProfileId }
            ?: _profiles.value.firstOrNull()
    }

    suspend fun select(id: String) = appConfig.setActiveProfile(id)

    suspend fun activeNodes(): List<Node> = activeProfile()?.nodes().orEmpty()

    suspend fun import(url: String, name: String): Result<Profile> = withContext(Dispatchers.IO) {
        runCatching {
            val nodes = subscriptions.fetch(url.trim())
            require(nodes.isNotEmpty()) { "no usable proxies in subscription" }
            val record = ProfileRecord(
                id = UUID.randomUUID().toString(),
                name = name.ifBlank { "Subscription ${_profiles.value.size + 1}" },
                subscribeUrl = url.trim(),
                nodesJson = json.encodeToString(nodes),
            )
            val updated = _profiles.value + record.toProfile()
            if (_profiles.value.isEmpty()) appConfig.setActiveProfile(record.id)
            save(updated)
            Logger.info("imported subscription with ${nodes.size} nodes")
            updated.last()
        }
    }

    suspend fun refresh(id: String): Result<Profile> = withContext(Dispatchers.IO) {
        runCatching {
            val current = _profiles.value.first { it.id == id }
            val url = current.subscribeUrl ?: error("profile has no subscription url")
            val nodes = subscriptions.fetch(url.trim())
            require(nodes.isNotEmpty()) { "no usable proxies in subscription" }
            val record = current.toRecord().copy(nodesJson = json.encodeToString(nodes))
            val updated = _profiles.value.map { if (it.id == id) record.toProfile() else it }
            save(updated)
            Logger.info("refreshed subscription $id (${nodes.size} nodes)")
            updated.first { it.id == id }
        }
    }

    suspend fun delete(id: String) {
        save(_profiles.value.filterNot { it.id == id })
    }

    /**
     * Runtime rule dir (filesDir/rules). The real rule sets live in `shared/rules`
     * at the repo root and should be copied into assets/ at build time; until then
     * placeholders keep the config loadable. This matches the shared rule set
     * (direct / proxy / media / ai / ad) used by Windows.
     */
    fun rulesDir(): String {
        val dir = File(context.filesDir, "rules")
        if (!dir.exists()) dir.mkdirs()
        ensureRuleFiles(dir)
        return dir.absolutePath
    }

    private fun ensureRuleFiles(dir: File) {
        val names = listOf("direct", "proxy", "media", "ai", "ad")
        if (names.all { File(dir, "$it.yaml").exists() }) return
        copyYamlFromAssets(dir, names)
        if (!names.all { File(dir, "$it.yaml").exists() }) {
            writePlaceholderRules(dir, names)
        }
    }

    private fun copyYamlFromAssets(dir: File, names: List<String>) {
        val copied = names.any { name ->
            try {
                context.assets.open("rules/$name.yaml").use { input ->
                    File(dir, "$name.yaml").writeBytes(input.readBytes())
                }
                true
            } catch (e: Exception) {
                false
            }
        }
        if (copied) Logger.info("bundled rule sets copied into data dir")
    }

    private fun writePlaceholderRules(dir: File, names: List<String>) {
        names.forEach { name ->
            val f = File(dir, "$name.yaml")
            if (!f.exists()) f.writeText("# placeholder $name rules — replaced from shared/rules\n")
        }
        Logger.warn("built-in rules not bundled yet; using placeholders")
    }

    private fun save(updated: List<Profile>) {
        storeFile.writeText(json.encodeToString(updated.map { it.toRecord() }))
        _profiles.value = updated
    }

    private fun load() {
        val records = runCatching { json.decodeFromString<List<ProfileRecord>>(storeFile.readText()) }
            .getOrDefault(emptyList())
        _profiles.value = records.map { it.toProfile() }
    }

    private fun Profile.nodes(): List<Node> = runCatching {
        json.decodeFromString<List<Node>>(content)
    }.getOrDefault(emptyList())

    private fun Profile.toRecord(): ProfileRecord =
        ProfileRecord(id, name, subscribeUrl, json.encodeToString(nodes()))

    private fun ProfileRecord.toProfile(): Profile =
        Profile(id, name, subscribeUrl, content = nodesJson)
}
