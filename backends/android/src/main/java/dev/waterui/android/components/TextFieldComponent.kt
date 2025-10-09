package dev.waterui.android.components

import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import dev.waterui.android.reactive.WuiBinding
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiAnyView
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val textFieldTypeId: WuiTypeId by lazy { NativeBindings.waterui_text_field_id().toTypeId() }

private val textFieldRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_text_field(node.rawPtr) }
    val binding = remember(struct.valuePtr) {
        WuiBinding.str(struct.valuePtr, env).also { it.watch() }
    }
    val valueState = binding.state
    val promptText = remember(struct.promptPtr) {
        // TODO: convert WuiText -> Compose Text properly.
        "TODO(prompt)"
    }
    val labelNode = remember(struct.labelPtr) { struct.labelPtr }
    val currentEnv = rememberUpdatedState(env)

    DisposableEffect(binding) {
        onDispose { binding.close() }
    }

    TextField(
        value = valueState.value,
        onValueChange = { binding.set(it) },
        label = {
            WuiAnyView(pointer = labelNode, environment = currentEnv.value)
        },
        placeholder = { Text(promptText) }
    )
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiTextField() {
    register({ textFieldTypeId }, textFieldRenderer)
}
