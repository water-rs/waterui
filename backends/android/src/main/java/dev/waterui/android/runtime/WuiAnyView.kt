package dev.waterui.android.runtime

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import dev.waterui.android.components.WuiRenderer

/**
 * Compose entry point that renders an opaque `AnyView` from the Rust view tree.
 */
@Composable
fun WuiAnyView(
    pointer: Long,
    environment: WuiEnvironment,
    registry: RenderRegistry = remember { RenderRegistry.default() }
) {
    val typeId = remember(pointer) { NativeBindings.waterui_view_id(pointer).toTypeId() }
    val node = remember(pointer, typeId) { WuiNode(pointer, typeId) }
    val currentRegistry = rememberUpdatedState(registry)
    val renderer: WuiRenderer? = remember(typeId, currentRegistry.value) {
        currentRegistry.value.resolve(typeId)
    }

    if (renderer != null) {
        renderer(node, environment)
    } else {
        val fallbackPtr = remember(pointer) {
            NativeBindings.waterui_view_body(pointer, environment.raw()).takeIf { it != 0L }
        }
        if (fallbackPtr != null) {
            WuiAnyView(pointer = fallbackPtr, environment = environment, registry = registry)
        } else {
            MissingComponent(typeId)
        }
    }
}

@Composable
private fun MissingComponent(typeId: WuiTypeId) {
    Text("TODO: Missing component for typeId=${'$'}typeId")
}
