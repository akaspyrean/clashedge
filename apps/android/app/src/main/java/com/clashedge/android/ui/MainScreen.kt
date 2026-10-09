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
                // T9：底部导航收敛为 首页 / 代理 / 更多（连接页尚不存在，
                // 落地后插入第 4 项）。订阅 / 日志 / 设置 收纳进「更多」。
                val items = listOf(
                    R.string.nav_home to "home",
                    R.string.nav_proxies to "proxies",
                    R.string.nav_more to "more",
                )
                items.forEachIndexed { index, (labelRes, key) ->
                    val selected = if (key == "more") tab >= 2 else tab == index
                    NavigationBarItem(
                        selected = selected,
                        onClick = { tab = if (key == "more") 2 else index },
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
            // 2 = 更多菜单；3/4/5 = 更多内的订阅 / 日志 / 设置（更多项保持选中）
            2 -> MoreScreen(modifier = contentModifier, onSelect = { next -> tab = next })
            3 -> ProfilesScreen(modifier = contentModifier, viewModel = viewModel,
                profiles = viewModel.profiles)
            4 -> LogsScreen(modifier = contentModifier, viewModel = viewModel,
                logs = viewModel.logs)
            5 -> SettingsScreen(modifier = contentModifier, viewModel = viewModel)
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
internal fun iconFor(key: String): androidx.compose.ui.graphics.vector.ImageVector =
    when (key) {
        "proxies" -> androidx.compose.material.icons.Icons.Filled.List
        "profiles" -> androidx.compose.material.icons.Icons.Filled.Person
        "logs" -> androidx.compose.material.icons.Icons.Filled.Info
        "settings" -> androidx.compose.material.icons.Icons.Filled.Settings
        "more" -> androidx.compose.material.icons.Icons.Filled.Menu
        else -> androidx.compose.material.icons.Icons.Filled.Home
    }
