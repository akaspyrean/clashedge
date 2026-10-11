// AUTO-GENERATED — do not edit.
// Source: design/tokens.json  ·  Generator: scripts/tokens/build.mjs  ·  Run: npm run tokens (apps/windows)
package com.clashedge.android.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

object CeTokens {
    val radiusXs: Dp = 6.dp
    val radiusSm: Dp = 8.dp
    val radiusMd: Dp = 12.dp
    val radiusLg: Dp = 16.dp
    val radiusXl: Dp = 20.dp
    val radiusFull: Dp = 999.dp

    interface Palette {
        val bgPage: Color
        val bgSurface: Color
        val bgSidebar: Color
        val bgElevated: Color
        val fillHover: Color
        val fillSoft: Color
        val border: Color
        val divider: Color
        val textPrimary: Color
        val textSecondary: Color
        val textTertiary: Color
        val textDisabled: Color
        val fillDisabled: Color
        val accentBg: Color
        val accentBgHover: Color
        val accentBgPress: Color
        val accentFg: Color
        val accentSoft: Color
        val onAccent: Color
        val successSolid: Color
        val successFg: Color
        val successSoft: Color
        val warningSolid: Color
        val warningFg: Color
        val warningSoft: Color
        val dangerSolid: Color
        val dangerFg: Color
        val dangerSoft: Color
        val dangerBg: Color
        val pendingSolid: Color
        val pendingFg: Color
        val pendingSoft: Color
        val idle: Color
        val chartDown: Color
        val chartUp: Color
        val chartGrid: Color
        val switchOff: Color
        val toastBg: Color
        val toastText: Color
    }

    object Light : Palette {
        override val bgPage = Color(0xFFF5F6F8)
        override val bgSurface = Color(0xFFFFFFFF)
        override val bgSidebar = Color(0xFFFFFFFF)
        override val bgElevated = Color(0xFFFFFFFF)
        override val fillHover = Color(0xFFF5F6F8)
        override val fillSoft = Color(0xFFEDEFF2)
        override val border = Color(0xFFE2E5EA)
        override val divider = Color(0xFFEDEFF2)
        override val textPrimary = Color(0xFF15171B)
        override val textSecondary = Color(0xFF4A515C)
        override val textTertiary = Color(0xFF666E7B)
        override val textDisabled = Color(0xFFA3AAB6)
        override val fillDisabled = Color(0xFFE2E5EA)
        override val accentBg = Color(0xFF2465F0)
        override val accentBgHover = Color(0xFF1C55D4)
        override val accentBgPress = Color(0xFF1746B0)
        override val accentFg = Color(0xFF1C55D4)
        override val accentSoft = Color(0xFFEBF1FE)
        override val onAccent = Color(0xFFFFFFFF)
        override val successSolid = Color(0xFF1E9E5A)
        override val successFg = Color(0xFF157A46)
        override val successSoft = Color(0xFFE8F6EE)
        override val warningSolid = Color(0xFFF0A01E)
        override val warningFg = Color(0xFF9A5700)
        override val warningSoft = Color(0xFFFEF4E2)
        override val dangerSolid = Color(0xFFE5484D)
        override val dangerFg = Color(0xFFCC2F35)
        override val dangerSoft = Color(0xFFFDECEC)
        override val dangerBg = Color(0xFFCC2F35)
        override val pendingSolid = Color(0xFF7B5CF5)
        override val pendingFg = Color(0xFF6542E6)
        override val pendingSoft = Color(0xFFF1EDFE)
        override val idle = Color(0xFF4A515C)
        override val chartDown = Color(0xFF2465F0)
        override val chartUp = Color(0xFF0891B2)
        override val chartGrid = Color(0xFFEDEFF2)
        override val switchOff = Color(0xFF7D8592)
        override val toastBg = Color(0xFF2B2F36)
        override val toastText = Color(0xFFFFFFFF)
    }

    object Dark : Palette {
        override val bgPage = Color(0xFF0F1115)
        override val bgSurface = Color(0xFF171A1F)
        override val bgSidebar = Color(0xFF171A1F)
        override val bgElevated = Color(0xFF1F2329)
        override val fillHover = Color(0xFF1F2329)
        override val fillSoft = Color(0xFF2A2F36)
        override val border = Color(0xFF2A2F36)
        override val divider = Color(0xFF2A2F36)
        override val textPrimary = Color(0xFFECEEF1)
        override val textSecondary = Color(0xFFA9B0BB)
        override val textTertiary = Color(0xFF8B93A0)
        override val textDisabled = Color(0xFF5A616C)
        override val fillDisabled = Color(0xFF2A2F36)
        override val accentBg = Color(0xFF2465F0)
        override val accentBgHover = Color(0xFF1C55D4)
        override val accentBgPress = Color(0xFF1746B0)
        override val accentFg = Color(0xFF5B92FF)
        override val accentSoft = Color(0xFF16243F)
        override val onAccent = Color(0xFFFFFFFF)
        override val successSolid = Color(0xFF3FCB7E)
        override val successFg = Color(0xFF3FCB7E)
        override val successSoft = Color(0xFF122A1E)
        override val warningSolid = Color(0xFFF5B544)
        override val warningFg = Color(0xFFF5B544)
        override val warningSoft = Color(0xFF2E2412)
        override val dangerSolid = Color(0xFFFF6B6F)
        override val dangerFg = Color(0xFFFF6B6F)
        override val dangerSoft = Color(0xFF331A1C)
        override val dangerBg = Color(0xFFCC2F35)
        override val pendingSolid = Color(0xFFA08CFF)
        override val pendingFg = Color(0xFFA08CFF)
        override val pendingSoft = Color(0xFF231D3D)
        override val idle = Color(0xFFA9B0BB)
        override val chartDown = Color(0xFF4A80F5)
        override val chartUp = Color(0xFF19A6B5)
        override val chartGrid = Color(0xFF2A2F36)
        override val switchOff = Color(0xFF3A4049)
        override val toastBg = Color(0xFFCDD2D9)
        override val toastText = Color(0xFF15171B)
    }
}
