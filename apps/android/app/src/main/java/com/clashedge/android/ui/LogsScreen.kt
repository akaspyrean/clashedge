package com.clashedge.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.clashedge.android.model.LogEntry
import kotlinx.coroutines.flow.StateFlow

@Composable
fun LogsScreen(
    modifier: Modifier = Modifier,
    viewModel: MainViewModel,
    logs: StateFlow<List<LogEntry>>,
) {
    val entries by logs.collectAsState()
    Column(modifier = modifier.fillMaxSize().padding(16.dp)) {
        Text("Logs", style = MaterialTheme.typography.titleLarge)
        LazyColumn(modifier = Modifier.fillMaxSize()) {
            items(entries.reversed(), key = { "${it.time}:${it.message.hashCode()}" }) { e ->
                Text(
                    text = "[${e.level}] ${e.message}",
                    modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
                    fontFamily = FontFamily.Monospace,
                    fontWeight = if (e.level == "ERROR") FontWeight.Bold else FontWeight.Normal,
                    color = when (e.level) {
                        "ERROR" -> MaterialTheme.colorScheme.error
                        "WARN" -> MaterialTheme.colorScheme.tertiary
                        else -> MaterialTheme.colorScheme.onSurface
                    },
                )
            }
        }
    }
}
