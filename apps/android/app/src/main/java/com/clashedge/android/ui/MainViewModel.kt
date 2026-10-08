package com.clashedge.android.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.clashedge.android.ClashEdgeApp
import com.clashedge.android.config.AppSettings
import com.clashedge.android.model.ConnectionState
import com.clashedge.android.model.LogEntry
import com.clashedge.android.model.Profile
import com.clashedge.android.model.ProxyGroup
import com.clashedge.android.util.Logger
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

class MainViewModel(application: Application) : AndroidViewModel(application) {

    private val app get() = application as ClashEdgeApp

    val state: StateFlow<ConnectionState> = app.coordinator.state
    val groups: StateFlow<List<ProxyGroup>> = app.coordinator.groups
    val error: StateFlow<String?> = app.coordinator.error
    val profiles: StateFlow<List<Profile>> = app.profiles.profiles
    val logs: StateFlow<List<LogEntry>> = Logger.entries
    val settings: Flow<AppSettings> = app.appConfig.settings

    fun setMode(mode: String) = app.coordinator.setMode(mode)
    fun selectProxy(group: String, node: String) = app.coordinator.selectProxy(group, node)

    fun importSubscription(url: String, name: String) = viewModelScope.launch {
        app.profiles.import(url, name).onFailure { Logger.error(it.message ?: "import failed") }
    }

    fun refreshProfile(id: String) = viewModelScope.launch {
        app.profiles.refresh(id).onFailure { Logger.error(it.message ?: "refresh failed") }
    }

    fun selectProfile(id: String) = viewModelScope.launch { app.profiles.select(id) }

    fun setLocale(locale: String) = viewModelScope.launch { app.appConfig.setLocale(locale) }

    fun setDarkTheme(enabled: Boolean) = viewModelScope.launch { app.appConfig.setDarkTheme(enabled) }
}
