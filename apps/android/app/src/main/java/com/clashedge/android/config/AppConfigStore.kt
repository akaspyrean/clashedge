package com.clashedge.android.config

import android.content.Context
import androidx.datastore.preferences.preferencesDataStore
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

private val Context.dataStore by preferencesDataStore(name = "clashedge_settings")

/** User-facing app settings (mode / locale / theme / active profile). */
data class AppSettings(
    val mode: String = "rule",
    val locale: String = "system",
    val darkTheme: Boolean? = null,
    val activeProfileId: String? = null,
) {
    companion object {
        val MODE_RULE = "rule"
        val MODE_GLOBAL = "global"
        val MODE_DIRECT = "direct"
    }
}

class AppConfigStore(private val context: Context) {

    private val settingMode = stringPreferencesKey("mode")
    private val settingLocale = stringPreferencesKey("locale")
    private val settingDark = stringPreferencesKey("darkTheme")
    private val settingActive = stringPreferencesKey("activeProfileId")

    val settings: Flow<AppSettings> = context.dataStore.data.map { p ->
        AppSettings(
            mode = p[settingMode] ?: AppSettings.MODE_RULE,
            locale = p[settingLocale] ?: "system",
            darkTheme = p[settingDark]?.toBoolean(),
            activeProfileId = p[settingActive],
        )
    }

    suspend fun setMode(mode: String) = context.dataStore.edit { it[settingMode] = mode }
    suspend fun setLocale(locale: String) = context.dataStore.edit { it[settingLocale] = locale }
    suspend fun setDarkTheme(enabled: Boolean) = context.dataStore.edit { it[settingDark] = enabled.toString() }
    suspend fun setActiveProfile(id: String) = context.dataStore.edit { it[settingActive] = id }
}
