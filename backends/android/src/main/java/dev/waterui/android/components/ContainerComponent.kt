package dev.waterui.android.components

import androidx.compose.runtime.remember
import dev.waterui.android.layout.ChildDescriptor
import dev.waterui.android.layout.RustLayout
import dev.waterui.android.layout.toChildDescriptors
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val containerTypeId: WuiTypeId by lazy {
    NativeBindings.waterui_container_id().toTypeId()
}

private val containerRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_container(node.rawPtr) }
    val descriptors: List<ChildDescriptor> = remember(struct) {
        struct.children.toChildDescriptors()
    }
    RustLayout(layoutPtr = struct.layoutPtr, descriptors = descriptors, environment = env) {
        // TODO: Iterate over child pointers and emit WuiAnyView for each child.
    }
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiContainer() {
    register({ containerTypeId }, containerRenderer)
}
