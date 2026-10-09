// theme/Theme.kt — Material 3 主题（T9：全部取值来自 Tokens.kt，不启用 dynamicColor）
package com.clashedge.android.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider

private val LightColors = lightColorScheme(
    primary = CeTokens.Light.accentFg, // #1C55D4
    onPrimary = CeTokens.Light.onAccent,
    primaryContainer = CeTokens.Light.accentSoft, // #EBF1FE
    onPrimaryContainer = CeTokens.Light.accentFg,
    secondary = CeTokens.Light.textSecondary,
    onSecondary = CeTokens.Light.onAccent,
    tertiary = CeTokens.Light.successFg,
    background = CeTokens.Light.bgPage, // #F5F6F8
    onBackground = CeTokens.Light.textPrimary,
    surface = CeTokens.Light.bgSurface,
    onSurface = CeTokens.Light.textPrimary,
    surfaceVariant = CeTokens.Light.fillSoft,
    onSurfaceVariant = CeTokens.Light.textSecondary,
    outline = CeTokens.Light.border, // #E2E5EA
    outlineVariant = CeTokens.Light.divider,
    error = CeTokens.Light.dangerFg,
    onError = CeTokens.Light.onAccent,
    errorContainer = CeTokens.Light.dangerSoft,
    onErrorContainer = CeTokens.Light.dangerFg,
)

private val DarkColors = darkColorScheme(
    primary = CeTokens.Dark.accentFg, // #5B92FF
    onPrimary = CeTokens.Dark.bgPage, // #0F1115
    primaryContainer = CeTokens.Dark.accentSoft,
    onPrimaryContainer = CeTokens.Dark.accentFg,
    secondary = CeTokens.Dark.textSecondary,
    onSecondary = CeTokens.Dark.bgPage,
    tertiary = CeTokens.Dark.successFg,
    background = CeTokens.Dark.bgPage, // #0F1115
    onBackground = CeTokens.Dark.textPrimary,
    surface = CeTokens.Dark.bgSurface, // #171A1F
    onSurface = CeTokens.Dark.textPrimary,
    surfaceVariant = CeTokens.Dark.bgElevated, // #1F2329
    onSurfaceVariant = CeTokens.Dark.textSecondary,
    outline = CeTokens.Dark.border,
    outlineVariant = CeTokens.Dark.divider,
    error = CeTokens.Dark.dangerFg,
    onError = CeTokens.Dark.bgPage,
    errorContainer = CeTokens.Dark.dangerSoft,
    onErrorContainer = CeTokens.Dark.dangerFg,
)

// Shapes：small 12 / medium 16 / extraLarge 20（§3.3）；未指定的档位同用 Token 值补齐。
private val CeShapes = Shapes(
    extraSmall = RoundedCornerShape(CeTokens.radiusXs),
    small = RoundedCornerShape(CeTokens.radiusMd),
    medium = RoundedCornerShape(CeTokens.radiusLg),
    large = RoundedCornerShape(CeTokens.radiusLg),
    extraLarge = RoundedCornerShape(CeTokens.radiusXl),
)

@Composable
fun ClashEdgeTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    // 不启用 dynamicColor：配色完全来自 design tokens，保证与 Windows 端一致。
    CompositionLocalProvider(LocalCeStatus provides ceStatusPalette(darkTheme)) {
        MaterialTheme(
            colorScheme = if (darkTheme) DarkColors else LightColors,
            shapes = CeShapes,
            content = content,
        )
    }
}
