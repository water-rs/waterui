package dev.waterui.android.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.remember
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val colorTypeId: WuiTypeId by lazy { NativeBindings.waterui_color_id().toTypeId() }

private val colorRenderer = WuiRenderer { node, _ ->
    val color = remember(node) {
        // TODO: Resolve actual color from computed pointer.
        androidx.compose.ui.graphics.Color.Magenta
    }
    Box(
        modifier = androidx.compose.ui.Modifier
            .fillMaxSize()
            .background(color)
    )
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiColor() {
    register({ colorTypeId }, colorRenderer)
}
