package dev.waterui.android.reference

import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateValue
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.VectorConverter
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.GenericShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.sin

private fun stressCardColor(index: Int): Color = when (index % 6) {
    0 -> Color(0xFF2563EB)
    1 -> Color(0xFF7C3AED)
    2 -> Color(0xFF0EA5E9)
    3 -> Color(0xFF059669)
    4 -> Color(0xFFD97706)
    else -> Color(0xFFDC2626)
}

/**
 * Compose twin of the stress fixture: three pressure sections whose tile
 * counts and toggle cadences come from the same WATERUI_STRESS_* environment
 * contract (delivered as waterui.env.* launch extras).
 */
@Composable
fun StressTwin() {
    val systemCount = AppEnv.int("WATERUI_STRESS_SYSTEM_COUNT", 120).coerceIn(18, 1440)
    val customCount = AppEnv.int("WATERUI_STRESS_CUSTOM_COUNT", 96).coerceIn(14, 1120)
    val filterCount = AppEnv.int("WATERUI_STRESS_FILTER_COUNT", 144).coerceIn(12, 1200)
    val toggleMs = AppEnv.int("WATERUI_STRESS_TOGGLE_MS", 680).coerceAtLeast(120)
    val filterToggleMs = AppEnv.int("WATERUI_STRESS_FILTER_TOGGLE_MS", 240).coerceAtLeast(80)

    var systemToggle by remember { mutableStateOf(false) }
    var filterPhase by remember { mutableStateOf(false) }

    LaunchedEffect(toggleMs) {
        while (true) {
            systemToggle = !systemToggle
            delay(toggleMs.toLong())
        }
    }
    LaunchedEffect(filterToggleMs) {
        while (true) {
            filterPhase = !filterPhase
            delay(filterToggleMs.toLong())
        }
    }

    val blurTarget = if (filterPhase) 10.5f else 0.8f
    val saturationTarget = if (filterPhase) 1.85f else 0.45f
    val hueTarget = if (filterPhase) 260f else 30f

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("WaterUI Stress App")
        CaptionText(
            "SYSTEM=$systemCount, CUSTOM=$customCount, FILTER=$filterCount, " +
                "TOGGLE=${toggleMs}ms/${filterToggleMs}ms",
        )
        HorizontalDivider()

        SystemSection(systemCount, systemToggle)
        HorizontalDivider()
        CustomSection(customCount)
        HorizontalDivider()
        FilterSection(filterCount, blurTarget, saturationTarget, hueTarget)
    }
}

@Composable
private fun SystemSection(count: Int, toggle: Boolean) {
    val columns = 18
    val rows = ceil(count.toDouble() / columns).toInt()
    Column(
        Modifier.fillMaxWidth().testTag("stress:system"),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        HeadlineText("System Animation Pressure")
        BodyText("Metadata animations offloaded to native animation engine")
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            repeat(rows) { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    repeat(columns) { col ->
                        val i = row * columns + col
                        if (i < count) SystemTile(toggle, i)
                    }
                }
            }
        }
    }
}

@Composable
private fun SystemTile(toggle: Boolean, index: Int) {
    val amp = (index % 5) * 2f + 8f
    val hiScale = (index % 4) * 0.09f + 0.9f
    val loScale = (index % 3) * 0.08f + 0.45f
    val hiRot = (index % 8) * 22.5f

    val scale = if (toggle) hiScale else loScale
    val rotation = if (toggle) hiRot else -hiRot
    val offsetX = if (toggle) amp else -amp

    Box(
        Modifier.size(24.dp)
            .graphicsLayer {
                scaleX = scale; scaleY = scale
                rotationZ = rotation
                translationX = offsetX * density / 4f
            }
            .clip(RoundedCornerShape(22))
            .background(stressCardColor(index)),
    )
}

@Composable
private fun CustomSection(count: Int) {
    val columns = 14
    val rows = ceil(count.toDouble() / columns).toInt()
    Column(
        Modifier.fillMaxWidth().testTag("stress:custom"),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        HeadlineText("Custom Animation Pressure")
        BodyText("Renderer-side geometry morph animation (framework-driven)")
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            repeat(rows) { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    repeat(columns) { col ->
                        val i = row * columns + col
                        if (i < count) CustomTile(i)
                    }
                }
            }
        }
    }
}

@Composable
private fun CustomTile(index: Int) {
    val duration = 520 + (index % 7) * 110
    val transition = rememberInfiniteTransition(label = "morph$index")
    val fraction by transition.animateFloat(
        initialValue = 0f, targetValue = 1f,
        animationSpec = infiniteRepeatable(
            tween(duration, easing = EaseInOut), RepeatMode.Reverse,
        ),
        label = "f$index",
    )
    val shape = remember(fraction) {
        if (fraction < 0.5f) CircleShape
        else RoundedCornerShape(percent = 26)
    }
    Box(
        Modifier.size(42.dp).clip(shape).background(stressCardColor(index)),
    )
}

@Composable
private fun FilterSection(
    count: Int,
    blurTarget: Float,
    saturationTarget: Float,
    hueTarget: Float,
) {
    val columns = 12
    val rows = ceil(count.toDouble() / columns).toInt()
    Column(
        Modifier.fillMaxWidth().testTag("stress:filter"),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        HeadlineText("Filter Pressure")
        BodyText("Filter interpolation + effect rendering with retargeting")
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            repeat(rows) { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    repeat(columns) { col ->
                        val i = row * columns + col
                        if (i < count) {
                            FilterTile(blurTarget, saturationTarget, hueTarget, i)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun FilterTile(
    blurTarget: Float,
    saturationTarget: Float,
    hueTarget: Float,
    index: Int,
) {
    val idx = index.toDouble()
    val blur = (sin(idx * 0.17) * 1.6 + blurTarget).coerceIn(0.0, 12.0).toFloat()
    val saturation = (cos(idx * 0.11) * 0.35 + saturationTarget).coerceIn(0.0, 2.0).toFloat()
    val hue = (((idx * 11 + hueTarget) % 360.0 + 360.0) % 360.0).toFloat()

    Box(
        Modifier.size(62.dp, 44.dp)
            .blur(blur.dp.coerceAtMost(12.dp))
            .graphicsLayer {
                colorFilter = ColorFilter.colorMatrix(
                    colorMatrixTimes(saturationMatrix(saturation), hueMatrix(hue)),
                )
            }
            .background(stressCardColor(index)),
    ) {
        CaptionText("FX")
    }
}
