// ui/MoreScreen.kt — 「更多」菜单（T9）
// 订阅 / 日志 / 设置 的入口收纳于此；连接页（Connections）尚不存在，
// 待其落地后按任务卡插入底部导航第 4 项。
package com.clashedge.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.clashedge.android.R

@Composable
fun MoreScreen(
    modifier: Modifier = Modifier,
    onSelect: (tab: Int) -> Unit,
) {
    // tab 索引与 MainScreen 的 when(tab) 对应：3=订阅 4=日志 5=设置
    val entries = listOf(
        R.string.nav_profiles to "profiles" to 3,
        R.string.nav_logs to "logs" to 4,
        R.string.nav_settings to "settings" to 5,
    )
    Column(
        modifier = modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
    ) {
        entries.forEach { (labelRes, key, tabIndex) ->
            ListItem(
                headlineContent = { Text(stringResource(labelRes)) },
                leadingContent = { Icon(iconFor(key), contentDescription = null) },
                modifier = Modifier
                    .padding(horizontal = 8.dp)
                    .clickable { onSelect(tabIndex) },
            )
        }
    }
}
