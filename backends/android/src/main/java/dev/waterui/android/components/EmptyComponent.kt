package dev.waterui.android.components

import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val emptyTypeId: WuiTypeId by lazy {
    NativeBindings.waterui_empty_id().toTypeId()
}

private val emptyRenderer = WuiRenderer { _, _ ->
    // TODO: Render nothing until we confirm desired behaviour.
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiEmptyView() {
    register({ emptyTypeId }, emptyRenderer)
}
