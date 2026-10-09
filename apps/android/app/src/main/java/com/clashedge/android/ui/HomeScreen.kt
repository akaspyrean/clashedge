package com.clashedge.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.clashedge.android.R
import com.clashedge.android.config.AppSettings
import com.clashedge.android.model.ConnectionState
import com.clashedge.android.ui.components.CePrimaryButton
import com.clashedge.android.ui.theme.CeStatusTone
import com.clashedge.android.ui.theme.CeTokens
import com.clashedge.android.ui.theme.LocalCeStatus

// danger-bg 在两套主题中为同值（tokens.json）：停止按钮的填充底。
private val StopContainerColor = CeTokens.Light.dangerBg
private val OnAccentColor = CeTokens.Light.onAccent

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
        val (label, tone) = statusUi(state)
        // 状态三重编码：色点用 solid、文字用 fg（设计系统规则：*-solid 不作文字色）。
        Box(
            modifier = Modifier
                .size(24.dp)
                .background(tone.solid, androidx.compose.foundation.shape.CircleShape),
        )
        Text(label, color = tone.fg, style = MaterialTheme.typography.headlineSmall)

        if (error != null) {
            Text(
                text = error!!,
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium,
            )
        }

        val running = state == ConnectionState.RUNNING
        if (running) {
            Button(
                onClick = onStopProxy,
                colors = ButtonDefaults.buttonColors(
                    containerColor = StopContainerColor,
                    contentColor = OnAccentColor,
                ),
            ) {
                Text(stringResource(R.string.stop_proxy))
            }
        } else {
            CePrimaryButton(onClick = onStartProxy) {
                Text(stringResource(R.string.start_proxy))
            }
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

@Composable
private fun statusUi(state: ConnectionState): Pair<String, CeStatusTone> {
    val palette = LocalCeStatus.current
    return when (state) {
        ConnectionState.RUNNING ->
            stringResource(R.string.status_connected) to palette.running
        ConnectionState.STARTING ->
            stringResource(R.string.status_connecting) to palette.pending
        ConnectionState.STOPPING ->
            stringResource(R.string.status_stopping) to palette.pending
        ConnectionState.ERROR ->
            stringResource(R.string.status_error) to palette.error
        else ->
            stringResource(R.string.status_disconnected) to palette.idle
    }
}
