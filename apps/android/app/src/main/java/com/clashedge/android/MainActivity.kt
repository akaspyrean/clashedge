package com.clashedge.android

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.lifecycleScope
import com.clashedge.android.ui.MainScreen
import com.clashedge.android.ui.theme.ClashEdgeTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {

    private val coordinator get() = (application as ClashEdgeApp).coordinator
    private var pendingStart = false

    private val vpnPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
            val granted = result.resultCode == Activity.RESULT_OK
            if (granted && pendingStart) {
                pendingStart = false
                lifecycleScope.launch { coordinator.start() }
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            ClashEdgeTheme {
                MainScreen(
                    onStartProxy = { requestStart() },
                    onStopProxy = { coordinator.stop() },
                )
            }
        }
    }

    private fun requestStart() {
        val permissionIntent: Intent? = coordinator.vpnPermissionIntent()
        if (permissionIntent == null) {
            lifecycleScope.launch { coordinator.start() }
        } else {
            pendingStart = true
            vpnPermissionLauncher.launch(permissionIntent)
        }
    }
}
