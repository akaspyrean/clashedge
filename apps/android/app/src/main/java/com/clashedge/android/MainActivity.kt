package com.clashedge.android

import android.app.Activity
import android.content.Intent
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.Lifecycle
import androidx.compose.foundation.isSystemInDarkTheme
import com.clashedge.android.ui.MainScreen
import com.clashedge.android.ui.theme.ClashEdgeTheme
import kotlinx.coroutines.launch
import java.util.Locale

class MainActivity : ComponentActivity() {

    private val coordinator get() = (application as ClashEdgeApp).coordinator
    private val appConfig get() = (application as ClashEdgeApp).appConfig
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

        // Observe settings and pass darkTheme/locale to theme
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                appConfig.settings.collect { settings ->
                    val darkTheme = settings.darkTheme ?: isSystemInDarkTheme()
                    val config = Configuration().apply {
                        if (settings.locale != "system" && settings.locale.isNotBlank()) {
                            val targetLocale = Locale.forLanguageTag(settings.locale)
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                                setLocale(targetLocale)
                            } else {
                                locale = targetLocale
                            }
                        }
                    }
                    resources.updateConfiguration(config, resources.displayMetrics)
                    setContent {
                        ClashEdgeTheme(darkTheme = darkTheme) {
                            MainScreen(
                                onStartProxy = { requestStart() },
                                onStopProxy = { coordinator.stop() },
                            )
                        }
                    }
                }
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
