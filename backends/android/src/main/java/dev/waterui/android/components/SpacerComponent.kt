package dev.waterui.android.components

import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val spacerTypeId: WuiTypeId by lazy { NativeBindings.waterui_spacer_id().toTypeId() }

private val spacerRenderer = WuiRenderer { _, _ ->
    Spacer(modifier = Modifier.size(0.dp))
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiSpacer() {
    register({ spacerTypeId }, spacerRenderer)
}
