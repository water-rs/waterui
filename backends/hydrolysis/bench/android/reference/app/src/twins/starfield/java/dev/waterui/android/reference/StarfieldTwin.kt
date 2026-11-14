package dev.waterui.android.reference

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.unit.dp
import kotlin.random.Random

private const val STAR_COUNT = 400

private class Star(seed: Int) {
    private val rnd = Random(seed)
    var x = rnd.nextFloat() * 2f - 1f
    var y = rnd.nextFloat() * 2f - 1f
    var z = rnd.nextFloat() * 0.9f + 0.1f

    fun reset() {
        x = rnd.nextFloat() * 2f - 1f
        y = rnd.nextFloat() * 2f - 1f
        z = 1f
    }
}

/**
 * Compose twin of the starfield fixture: a dark 400x500 shader-style
 * starfield, continuously animated at display cadence — the GPU-pacing
 * workload the fixture produces on WaterUI.
 */
@Composable
fun StarfieldTwin() {
    val stars = remember { List(STAR_COUNT) { Star(it) } }
    var tick by remember { mutableFloatStateOf(0f) }

    LaunchedEffect(Unit) {
        while (true) {
            withFrameNanos { nanos ->
                tick = nanos / 1_000_000f
                for (star in stars) {
                    star.z -= 0.006f
                    if (star.z <= 0.05f) star.reset()
                }
            }
        }
    }

    Column(
        Modifier.fillMaxSize().padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        TitleText("Starfield")
        CaptionText("Rendered at 120fps — $STAR_COUNT procedural stars")
        Box(
            Modifier.fillMaxWidth().height(500.dp)
                .background(Color(0xFF06060F))
                .testTag("starfield:canvas"),
        ) {
            Canvas(Modifier.fillMaxSize()) {
                tick // subscribe the canvas to the animation tick
                val cx = size.width / 2f
                val cy = size.height / 2f
                for (star in stars) {
                    val px = cx + (star.x / star.z) * cx
                    val py = cy + (star.y / star.z) * cy
                    if (px in 0f..size.width && py in 0f..size.height) {
                        val depth = 1f - star.z
                        drawCircle(
                            color = Color.White.copy(alpha = depth),
                            radius = 1f + depth * 2.5f,
                            center = Offset(px, py),
                        )
                    }
                }
            }
        }
    }
}
