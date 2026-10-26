package com.clashedge.android

import android.app.Activity
import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.lifecycle.lifecycleScope
import com.clashedge.android.config.AppSettings
import com.clashedge.android.ui.MainScreen
import com.clashedge.android.ui.theme.ClashEdgeTheme
import java.util.Locale
import kotlinx.coroutines.launch

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

        // setContent is called exactly once; settings are observed as Compose state
        // inside the composition instead of re-invoking setContent from a coroutine.
        setContent {
            val settings by appConfig.settings.collectAsState(initial = AppSettings())
            val darkTheme = settings.darkTheme ?: isSystemInDarkTheme()

            ApplyAppLocale(settings.locale)

            ClashEdgeTheme(darkTheme = darkTheme) {
                MainScreen(
                    onStartProxy = { requestStart() },
                    onStopProxy = { coordinator.stop() },
                )
            }
        }
    }

    @Suppress("DEPRECATION") // resources.updateConfiguration is the only API that applies a locale to a running Activity
    @Composable
    private fun ApplyAppLocale(tag: String) {
        if (tag.isBlank() || tag == "system") return
        val target = Locale.forLanguageTag(tag)
        if (resources.configuration.locales[0].language == target.language) return
        LaunchedEffect(tag) {
            val config = Configuration(resources.configuration)
            config.setLocale(target)
            resources.updateConfiguration(config, resources.displayMetrics)
            recreate()
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
