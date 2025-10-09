package dev.waterui.android.components

import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import dev.waterui.android.layout.ChildDescriptor
import dev.waterui.android.layout.RustLayout
import dev.waterui.android.layout.toChildDescriptors
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.*

private val containerTypeId: WuiTypeId by lazy {
    NativeBindings.waterui_container_id().toTypeId()
}

private val containerRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_container(node.rawPtr) }
    val childPointers = remember(struct.childrenPtr) {
        if (struct.childrenPtr == 0L) emptyList() else NativeAnyViews(struct.childrenPtr).usePointer { it.toList() }
    }
    val descriptors: List<ChildDescriptor> = remember(childPointers) {
        childPointers.toChildDescriptors()
    }

    RustLayout(layoutPtr = struct.layoutPtr, descriptors = descriptors, environment = env) {
        childPointers.forEach { childPtr ->
            key(childPtr) {
                WuiAnyView(pointer = childPtr, environment = env)
            }
        }
    }
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiContainer() {
    register({ containerTypeId }, containerRenderer)
}
