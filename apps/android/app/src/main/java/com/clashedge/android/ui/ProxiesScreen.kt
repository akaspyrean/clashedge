package com.clashedge.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.clashedge.android.R
import com.clashedge.android.model.ProxyGroup
import kotlinx.coroutines.flow.StateFlow

@Composable
fun ProxiesScreen(
    modifier: Modifier = Modifier,
    viewModel: MainViewModel,
    groups: StateFlow<List<ProxyGroup>>,
) {
    val groupList by groups.collectAsState()
    Column(modifier = modifier.fillMaxSize().padding(16.dp)) {
        Text(stringResource(R.string.nav_proxies), style = MaterialTheme.typography.titleLarge)
        if (groupList.isEmpty()) {
            Text("No proxy groups yet.", modifier = Modifier.padding(top = 16.dp))
        } else {
            LazyColumn(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(12.dp)) {
                items(groupList, key = { it.name }) { group ->
                    ProxyGroupCard(group = group, onSelect = { _, node -> viewModel.selectProxy(group.name, node) })
                }
            }
        }
    }
}

@Composable
private fun ProxyGroupCard(
    group: ProxyGroup,
    onSelect: (String, String) -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(group.name, style = MaterialTheme.typography.titleMedium)
            Text(
                "${group.type} · now: ${group.now ?: "—"}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            group.nodes.forEach { node ->
                Text(
                    node,
                    modifier = Modifier.padding(top = 4.dp).fillMaxWidth(),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
    }
}
