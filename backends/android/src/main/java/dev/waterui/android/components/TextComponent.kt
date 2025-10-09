package dev.waterui.android.components

import androidx.compose.material3.Text
import androidx.compose.runtime.remember
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val textTypeId: WuiTypeId by lazy {
    NativeBindings.waterui_text_id().toTypeId()
}

private val textRenderer = WuiRenderer { node, _ ->
    val text = remember(node) {
        val struct = NativeBindings.waterui_force_as_text(node.rawPtr)
        // TODO: Convert styled string pointer into Compose AnnotatedString.
        "TODO(text)"
    }
    Text(text)
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiText() {
    register({ textTypeId }, textRenderer)
}
