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
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

class MainViewModel(application: Application) : AndroidViewModel(application) {

    private val app get() = getApplication<ClashEdgeApp>()

    val state: StateFlow<ConnectionState> = app.coordinator.state
    val groups: StateFlow<List<ProxyGroup>> = app.coordinator.groups
    val error: StateFlow<String?> = app.coordinator.error
    val profiles: StateFlow<List<Profile>> = app.profiles.profiles
    val logs: StateFlow<List<LogEntry>> = Logger.entries
    val settings: Flow<AppSettings> = app.appConfig.settings

    // UI-visible operation errors (import/refresh/select profile etc.)
    private val _operationError = MutableStateFlow<String?>(null)
    val operationError: StateFlow<String?> = _operationError

    fun setMode(mode: String) = app.coordinator.setMode(mode)
    fun selectProxy(group: String, node: String) = app.coordinator.selectProxy(group, node)

    fun importSubscription(url: String, name: String) = viewModelScope.launch {
        _operationError.value = null
        app.profiles.import(url, name).onFailure { e ->
            _operationError.value = e.message ?: "import failed"
            Logger.error(e.message ?: "import failed")
        }
    }

    fun refreshProfile(id: String) = viewModelScope.launch {
        _operationError.value = null
        app.profiles.refresh(id).onFailure { e ->
            _operationError.value = e.message ?: "refresh failed"
            Logger.error(e.message ?: "refresh failed")
        }
    }

    fun selectProfile(id: String) = viewModelScope.launch {
        _operationError.value = null
        app.profiles.select(id).onFailure { e ->
            _operationError.value = e.message ?: "select profile failed"
            Logger.error(e.message ?: "select profile failed")
        }
    }

    fun restartCore() = viewModelScope.launch {
        _operationError.value = null
        try {
            app.coordinator.restartCore()
        } catch (e: Exception) {
            _operationError.value = e.message ?: "restart failed"
            Logger.error(e.message ?: "restart failed")
        }
    }

    fun reloadCore() = viewModelScope.launch {
        _operationError.value = null
        try {
            app.coordinator.reloadCore()
        } catch (e: Exception) {
            _operationError.value = e.message ?: "reload failed"
            Logger.error(e.message ?: "reload failed")
        }
    }

    fun setLocale(locale: String) = viewModelScope.launch { app.appConfig.setLocale(locale) }

    fun setDarkTheme(enabled: Boolean) = viewModelScope.launch { app.appConfig.setDarkTheme(enabled) }

    fun clearOperationError() { _operationError.value = null }
}
