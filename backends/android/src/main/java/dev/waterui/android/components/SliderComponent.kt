package dev.waterui.android.components

import androidx.compose.material3.Slider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import dev.waterui.android.reactive.WuiBinding
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiAnyView
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val sliderTypeId: WuiTypeId by lazy { NativeBindings.waterui_slider_id().toTypeId() }

private val sliderRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_slider(node.rawPtr) }
    val binding = remember(struct.bindingPtr) {
        WuiBinding.double(struct.bindingPtr, env).also { it.watch() }
    }
    val valueState = binding.state
    DisposableEffect(binding) {
        onDispose { binding.close() }
    }
    Slider(
        value = valueState.value.toFloat(),
        onValueChange = { binding.set(it.toDouble()) },
        valueRange = struct.rangeStart.toFloat()..struct.rangeEnd.toFloat()
    )
    WuiAnyView(pointer = struct.labelPtr, environment = env)
    WuiAnyView(pointer = struct.minLabelPtr, environment = env)
    WuiAnyView(pointer = struct.maxLabelPtr, environment = env)
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiSlider() {
    register({ sliderTypeId }, sliderRenderer)
}
