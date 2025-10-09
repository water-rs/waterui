package dev.waterui.android.runtime

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState

/**
 * Top-level composable that wires a [WuiEnvironment] into Compose and renders the root view.
 */
@Composable
fun WaterUIApplication(registry: RenderRegistry = RenderRegistry.default()) {
    val environment = remember {
        WuiEnvironment(NativeBindings.waterui_init())
    }

    val currentRegistry = rememberUpdatedState(registry)

    DisposableEffect(environment) {
        onDispose { environment.close() }
    }

    val rootPtr = remember(environment) {
        NativeBindings.waterui_main(environment.raw())
    }

    WuiAnyView(pointer = rootPtr, environment = environment, registry = currentRegistry.value)
}
