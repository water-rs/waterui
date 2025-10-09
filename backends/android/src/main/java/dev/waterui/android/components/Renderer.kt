package dev.waterui.android.components

import androidx.compose.runtime.Composable
import dev.waterui.android.runtime.WuiEnvironment
import dev.waterui.android.runtime.WuiNode
import dev.waterui.android.runtime.WuiTypeId

/**
 * Functional renderer for a WaterUI node. Wraps a composable lambda so we can store it in collections.
 */
fun interface WuiRenderer {
    @Composable
    operator fun invoke(node: WuiNode, env: WuiEnvironment)
}

/**
 * Convenience extension for populating the render registry map.
 */
fun MutableMap<WuiTypeId, WuiRenderer>.register(
    idProvider: () -> WuiTypeId,
    renderer: WuiRenderer
) {
    put(idProvider(), renderer)
}
