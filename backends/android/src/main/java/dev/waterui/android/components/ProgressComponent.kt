package dev.waterui.android.components

import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.runtime.remember
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val progressTypeId: WuiTypeId by lazy { NativeBindings.waterui_progress_id().toTypeId() }

private val progressRenderer = WuiRenderer { node, _ ->
    val struct = remember(node) { NativeBindings.waterui_force_as_progress(node.rawPtr) }
    // TODO: Hook up computed value for progress.
    when (struct.style) {
        0 -> LinearProgressIndicator()
        1 -> CircularProgressIndicator()
        else -> LinearProgressIndicator()
    }
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiProgress() {
    register({ progressTypeId }, progressRenderer)
}
