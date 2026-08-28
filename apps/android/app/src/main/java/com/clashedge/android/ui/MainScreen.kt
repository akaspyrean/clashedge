package com.clashedge.android.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.viewmodel.compose.viewModel
import com.clashedge.android.R

@Composable
fun MainScreen(
    onStartProxy: () -> Unit,
    onStopProxy: () -> Unit,
    viewModel: MainViewModel = viewModel(),
) {
    var tab by rememberSaveable { mutableIntStateOf(0) }

    Scaffold(
        bottomBar = {
            NavigationBar {
                val items = listOf(
                    R.string.nav_home to "home",
                    R.string.nav_proxies to "proxies",
                    R.string.nav_profiles to "profiles",
                    R.string.nav_logs to "logs",
                    R.string.nav_settings to "settings",
                )
                items.forEachIndexed { index, (labelRes, key) ->
                    NavigationBarItem(
                        selected = tab == index,
                        onClick = { tab = index },
                        icon = { Icon(iconFor(key), contentDescription = null) },
                        label = { Text(stringResource(labelRes)) },
                    )
                }
            }
        },
    ) { padding ->
        val contentModifier = Modifier.padding(padding)
        when (tab) {
            0 -> HomeScreen(
                modifier = contentModifier,
                viewModel = viewModel,
                onStartProxy = onStartProxy,
                onStopProxy = onStopProxy,
            )
            1 -> ProxiesScreen(modifier = contentModifier, viewModel = viewModel,
                groups = viewModel.groups)
            2 -> ProfilesScreen(modifier = contentModifier, viewModel = viewModel,
                profiles = viewModel.profiles)
            3 -> LogsScreen(modifier = contentModifier, viewModel = viewModel,
                logs = viewModel.logs)
            4 -> SettingsScreen(modifier = contentModifier, viewModel = viewModel)
            else -> HomeScreen(
                modifier = contentModifier,
                viewModel = viewModel,
                onStartProxy = onStartProxy,
                onStopProxy = onStopProxy,
            )
        }
    }
}

@Composable
private fun iconFor(key: String): androidx.compose.ui.graphics.vector.ImageVector = when (key) {
    "proxies" -> androidx.compose.material.icons.Icons.Filled.List
    "profiles" -> androidx.compose.material.icons.Icons.Filled.Person
    "logs" -> androidx.compose.material.icons.Icons.Filled.Info
    "settings" -> androidx.compose.material.icons.Icons.Filled.Settings
    else -> androidx.compose.material.icons.Icons.Filled.Home
}
