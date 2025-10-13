package dev.waterui.android.components

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.matchParentSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.RENDERER_BUFFER_FORMAT_RGBA8888
import dev.waterui.android.runtime.WuiEnvironment
import dev.waterui.android.runtime.WuiNode
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId
import kotlin.math.roundToInt

private val rendererViewTypeId: WuiTypeId by lazy {
    NativeBindings.waterui_renderer_view_id().toTypeId()
}

private val rendererViewRenderer = WuiRenderer { node, env ->
    RendererView(node = node, env = env)
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiRendererView() {
    register({ rendererViewTypeId }, rendererViewRenderer)
}

@Composable
private fun RendererView(node: WuiNode, env: WuiEnvironment) {
    val handle = remember(node.pointer) {
        NativeBindings.waterui_force_as_renderer_view(node.pointer)
    }

    DisposableEffect(handle) {
        onDispose { NativeBindings.waterui_drop_renderer_view(handle) }
    }

    val width = remember(handle) {
        NativeBindings.waterui_renderer_view_width(handle)
    }
    val height = remember(handle) {
        NativeBindings.waterui_renderer_view_height(handle)
    }

    var bitmap by remember(handle) { mutableStateOf<ImageBitmap?>(null) }

    LaunchedEffect(handle, width, height, env.raw()) {
        val targetWidth = width.roundToInt().coerceAtLeast(0)
        val targetHeight = height.roundToInt().coerceAtLeast(0)
        if (targetWidth == 0 || targetHeight == 0) {
            bitmap = null
            return@LaunchedEffect
        }
        val preferredFormat = NativeBindings.waterui_renderer_view_preferred_format(handle)
        if (preferredFormat != RENDERER_BUFFER_FORMAT_RGBA8888) {
            bitmap = null
            return@LaunchedEffect
        }
        val stride = targetWidth * 4
        val buffer = ByteArray(stride * targetHeight)
        val rendered = NativeBindings.waterui_renderer_view_render_cpu(
            handle,
            buffer,
            targetWidth,
            targetHeight,
            stride,
            RENDERER_BUFFER_FORMAT_RGBA8888
        )
        if (!rendered) {
            bitmap = null
            return@LaunchedEffect
        }

        val pixels = IntArray(targetWidth * targetHeight)
        var offset = 0
        for (index in pixels.indices) {
            val r = buffer[offset].toInt() and 0xFF
            val g = buffer[offset + 1].toInt() and 0xFF
            val b = buffer[offset + 2].toInt() and 0xFF
            val a = buffer[offset + 3].toInt() and 0xFF
            pixels[index] = (a shl 24) or (r shl 16) or (g shl 8) or b
            offset += 4
        }

        val androidBitmap = Bitmap.createBitmap(targetWidth, targetHeight, Bitmap.Config.ARGB_8888)
        androidBitmap.setPixels(pixels, 0, targetWidth, 0, 0, targetWidth, targetHeight)
        bitmap = androidBitmap.asImageBitmap()
    }

    Box(
        modifier = Modifier
            .background(Color.Transparent)
            .let {
                if (width > 0f && height > 0f) {
                    it.size(width.dp, height.dp)
                } else {
                    it
                }
            },
        contentAlignment = Alignment.Center
    ) {
        val image = bitmap
        if (image != null) {
            Image(
                bitmap = image,
                contentDescription = null,
                contentScale = ContentScale.FillBounds,
                modifier = Modifier.matchParentSize()
            )
        }
    }
}
