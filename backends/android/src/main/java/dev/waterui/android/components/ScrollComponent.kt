package dev.waterui.android.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.runtime.remember
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiAnyView
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val scrollTypeId: WuiTypeId by lazy { NativeBindings.waterui_scroll_view_id().toTypeId() }

private val scrollRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_scroll(node.rawPtr) }
    val children = remember(struct.contentPtr) { listOf(struct.contentPtr) }
    // TODO: Support axis selection and multi-child arrays once JNI exposes them.
    LazyColumn {
        items(children) { ptr ->
            Box {
                WuiAnyView(pointer = ptr, environment = env)
            }
        }
    }
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiScroll() {
    register({ scrollTypeId }, scrollRenderer)
}
