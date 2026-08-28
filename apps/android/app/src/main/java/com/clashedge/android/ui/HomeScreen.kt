package com.clashedge.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.clashedge.android.R
import com.clashedge.android.config.AppSettings
import com.clashedge.android.model.ConnectionState

@Composable
fun HomeScreen(
    modifier: Modifier = Modifier,
    viewModel: MainViewModel,
    onStartProxy: () -> Unit,
    onStopProxy: () -> Unit,
) {
    val state by viewModel.state.collectAsState()
    val error by viewModel.error.collectAsState()
    val settings by viewModel.settings.collectAsState(initial = AppSettings())

    Column(
        modifier = modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val (label, color) = statusUi(state)
        Box(modifier = Modifier.size(24.dp).background(color, androidx.compose.foundation.shape.CircleShape))
        Text(label, color = color, style = MaterialTheme.typography.headlineSmall)

        if (error != null) {
            Text(
                text = error!!,
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium,
            )
        }

        val running = state == ConnectionState.RUNNING
        Button(
            onClick = if (running) onStopProxy else onStartProxy,
            colors = ButtonDefaults.buttonColors(
                containerColor = if (running) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
            ),
        ) {
            Text(stringResource(if (running) R.string.stop_proxy else R.string.start_proxy))
        }

        Text("Mode / 模式", style = MaterialTheme.typography.titleMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(
                selected = settings.mode == AppSettings.MODE_RULE,
                onClick = { viewModel.setMode(AppSettings.MODE_RULE) },
                label = { Text(stringResource(R.string.mode_rule)) },
            )
            FilterChip(
                selected = settings.mode == AppSettings.MODE_GLOBAL,
                onClick = { viewModel.setMode(AppSettings.MODE_GLOBAL) },
                label = { Text(stringResource(R.string.mode_global)) },
            )
            FilterChip(
                selected = settings.mode == AppSettings.MODE_DIRECT,
                onClick = { viewModel.setMode(AppSettings.MODE_DIRECT) },
                label = { Text(stringResource(R.string.mode_direct)) },
            )
        }
    }
}

private fun statusUi(state: ConnectionState): Pair<String, Color> = when (state) {
    ConnectionState.RUNNING -> "Connected" to Color(0xFF15803D)
    ConnectionState.STARTING -> "Connecting…" to Color(0xFFD97706)
    ConnectionState.STOPPING -> "Stopping…" to Color(0xFFD97706)
    ConnectionState.ERROR -> "Error" to Color(0xFFB91C1C)
    else -> "Disconnected" to Color(0xFF6B7280)
}
