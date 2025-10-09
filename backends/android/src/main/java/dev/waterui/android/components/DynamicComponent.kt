package dev.waterui.android.components

import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiAnyView
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val dynamicTypeId: WuiTypeId by lazy { NativeBindings.waterui_dynamic_id().toTypeId() }

private val dynamicRenderer = WuiRenderer { node, env ->
    val state = remember { mutableStateOf(node.rawPtr) }
    val currentEnv = rememberUpdatedState(env)
    // TODO: wire watcher updates from JNI to refresh state.value.
    WuiAnyView(pointer = state.value, environment = currentEnv.value)
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiDynamic() {
    register({ dynamicTypeId }, dynamicRenderer)
}
