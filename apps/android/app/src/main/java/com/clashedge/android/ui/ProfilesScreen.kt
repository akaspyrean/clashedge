package com.clashedge.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.clashedge.android.R
import com.clashedge.android.model.Profile
import kotlinx.coroutines.flow.StateFlow

@Composable
fun ProfilesScreen(
    modifier: Modifier = Modifier,
    viewModel: MainViewModel,
    profiles: StateFlow<List<Profile>>,
) {
    val list by profiles.collectAsState()
    var showDialog by remember { mutableStateOf(false) }
    var url by remember { mutableStateOf("") }

    Column(modifier = modifier.fillMaxSize().padding(16.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = androidx.compose.foundation.layout.Arrangement.SpaceBetween,
        ) {
            Text(stringResource(R.string.nav_profiles), style = MaterialTheme.typography.titleLarge)
            Button(onClick = { showDialog = true }) {
                Text(stringResource(R.string.import_subscription))
            }
        }

        if (list.isNotEmpty()) {
            LazyColumn(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(12.dp)) {
                items(list, key = { it.id }) { profile ->
                    ProfileCard(
                        profile = profile,
                        onRefresh = { viewModel.refreshProfile(profile.id) },
                        onActivate = { viewModel.selectProfile(profile.id) },
                    )
                }
            }
        } else {
            Text("No subscriptions yet.", modifier = Modifier.padding(top = 16.dp))
        }
    }

    if (showDialog) {
        AlertDialog(
            onDismissRequest = { showDialog = false },
            title = { Text(stringResource(R.string.import_subscription)) },
            text = {
                OutlinedTextField(
                    value = url,
                    onValueChange = { url = it },
                    label = { Text(stringResource(R.string.subscription_url_hint)) },
                    singleLine = true,
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    viewModel.importSubscription(url, "Subscription")
                    showDialog = false
                    url = ""
                }) {
                    Text(stringResource(R.string.update_subscription))
                }
            },
            dismissButton = {
                TextButton(onClick = { showDialog = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun ProfileCard(
    profile: Profile,
    onRefresh: () -> Unit,
    onActivate: () -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(profile.name, style = MaterialTheme.typography.titleMedium)
            Text(
                profile.subscribeUrl ?: "local",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Row(modifier = Modifier.padding(top = 8.dp)) {
                TextButton(onClick = onActivate) { Text("Activate") }
                if (profile.subscribeUrl != null) {
                    TextButton(onClick = onRefresh) { Text(stringResource(R.string.update_subscription)) }
                }
            }
        }
    }
}
