// ui/components/CePrimaryButton.kt — 填充型主按钮（T9）
// containerColor = #2465F0 + 白字；accent-bg 在两套主题中为同值（tokens.json），
// 故直接引用 Token，不随主题切换。
package com.clashedge.android.ui.components

import androidx.compose.foundation.layout.RowScope
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import com.clashedge.android.ui.theme.CeTokens

@Composable
fun CePrimaryButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable RowScope.() -> Unit,
) {
    Button(
        onClick = onClick,
        modifier = modifier,
        colors = ButtonDefaults.buttonColors(
            containerColor = CeTokens.Light.accentBg, // #2465F0（Light/Dark 同值）
            contentColor = CeTokens.Light.onAccent,
        ),
        content = content,
    )
}
