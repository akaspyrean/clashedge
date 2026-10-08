package com.clashedge.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.clashedge.android.config.AppSettings

@Composable
fun SettingsScreen(
    modifier: Modifier = Modifier,
    viewModel: MainViewModel,
) {
    val settings by viewModel.settings.collectAsState(initial = AppSettings())

    Column(modifier = modifier.fillMaxSize().padding(16.dp)) {
        Text("Settings", style = MaterialTheme.typography.titleLarge)

        Spacer(modifier = Modifier.height(24.dp))

        Text("Language / 语言", style = MaterialTheme.typography.titleMedium)
        Row {
            TextButton(onClick = { viewModel.setLocale("zh") }) { Text("中文") }
            TextButton(onClick = { viewModel.setLocale("en") }) { Text("English") }
        }

        Spacer(modifier = Modifier.height(16.dp))

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Dark theme / 深色", modifier = Modifier.weight(1f))
            Switch(
                checked = settings.darkTheme == true,
                onCheckedChange = { viewModel.setDarkTheme(it) },
            )
        }

        Spacer(modifier = Modifier.height(24.dp))
        Text(
            "ClashEdge for Android · Lightweight / Simple / Mihomo-based",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
