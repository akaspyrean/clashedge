// theme/CeStatus.kt — 连接状态配色（T9）
// 通过 CompositionLocal 提供，替代 HomeScreen 中写死的状态色。
// 来源：design/tokens.json（--ce-success / warning / danger / pending / idle）。
package com.clashedge.android.ui.theme

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** 单个状态档位的三个色位：solid 用于色点/图标，fg 用于文字，soft 用于浅底。 */
data class CeStatusTone(val solid: Color, val fg: Color, val soft: Color)

/** 四类连接状态的调色板（对齐设计系统 §3.1 规则：文字一律用 *-fg）。 */
data class CeStatusPalette(
    val running: CeStatusTone,
    val pending: CeStatusTone,
    val error: CeStatusTone,
    val idle: CeStatusTone,
)

/** 状态配色入口：必须包在 ClashEdgeTheme 内使用。 */
val LocalCeStatus = staticCompositionLocalOf<CeStatusPalette> {
    error("CeStatusPalette not provided — wrap content in ClashEdgeTheme")
}

internal fun ceStatusPalette(dark: Boolean): CeStatusPalette {
    val t = if (dark) CeTokens.Dark else CeTokens.Light
    return CeStatusPalette(
        running = CeStatusTone(t.successSolid, t.successFg, t.successSoft),
        pending = CeStatusTone(t.pendingSolid, t.pendingFg, t.pendingSoft),
        error = CeStatusTone(t.dangerSolid, t.dangerFg, t.dangerSoft),
        idle = CeStatusTone(t.idle, t.idle, t.fillSoft),
    )
}
