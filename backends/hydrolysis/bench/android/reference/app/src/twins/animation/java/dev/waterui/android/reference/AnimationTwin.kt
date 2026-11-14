package dev.waterui.android.reference

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/**
 * Compose twin of the animation fixture: scale, rotation, translation,
 * combined transform, progress and spring-toggle sections plus the
 * curve-comparison and staggered demos — matching the fixture's controls
 * and spring/easing cadences.
 */
@Composable
fun AnimationTwin() {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Animations")
        ScaleSection()
        RotationSection()
        TranslationSection()
        CombinedSection()
        ProgressSection()
        SpringSection()
        CurvesSection()
        StaggeredSection()
    }
}

@Composable
private fun Box_(modifier: Modifier = Modifier, color: Color = MaterialTheme.colorScheme.primary) {
    Box(
        modifier.size(64.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(color),
    )
}

@Composable
private fun ScaleSection() {
    var target by remember { mutableFloatStateOf(1f) }
    val scale by animateFloatAsState(
        target,
        animationSpec = spring(dampingRatio = 0.5f, stiffness = Spring.StiffnessMedium),
        label = "scale",
    )
    Column(Modifier.testTag("anim:scale")) {
        HeadlineText("Scale (spring)")
        BodyText("Click buttons to scale the box with spring physics")
        Box_(Modifier.graphicsLayer { scaleX = scale; scaleY = scale })
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for ((label, v) in listOf("0.5x" to 0.5f, "1x" to 1f, "1.5x" to 1.5f, "2x" to 2f)) {
                Button(onClick = {
                    BenchMarkers.tap()
                    target = v
                }, modifier = Modifier.testTag("anim:scale-$label")) { Text(label) }
            }
        }
    }
}

@Composable
private fun RotationSection() {
    var rotation by remember { mutableFloatStateOf(0f) }
    val animated by animateFloatAsState(
        rotation,
        animationSpec = tween(400, easing = FastOutSlowInEasing),
        label = "rotation",
    )
    Column(Modifier.testTag("anim:rotation")) {
        HeadlineText("Rotation")
        Box_(Modifier.graphicsLayer { rotationZ = animated })
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { BenchMarkers.tap(); rotation -= 90f }) { Text("-90°") }
            Button(onClick = { BenchMarkers.tap(); rotation -= 45f }) { Text("-45°") }
            Button(onClick = { BenchMarkers.tap(); rotation = 0f }) { Text("Reset") }
            Button(onClick = { BenchMarkers.tap(); rotation += 45f }) { Text("+45°") }
            Button(onClick = { BenchMarkers.tap(); rotation += 90f }) { Text("+90°") }
        }
    }
}

@Composable
private fun TranslationSection() {
    var x by remember { mutableFloatStateOf(0f) }
    var y by remember { mutableFloatStateOf(0f) }
    val ax by animateFloatAsState(x, label = "tx")
    val ay by animateFloatAsState(y, label = "ty")
    Column(Modifier.testTag("anim:translation")) {
        HeadlineText("Translation")
        Box_(Modifier.graphicsLayer { translationX = ax; translationY = ay })
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { BenchMarkers.tap(); x = 0f; y = 0f }) { Text("Center") }
            Button(onClick = { BenchMarkers.tap(); x = -200f }) { Text("Left") }
            Button(onClick = { BenchMarkers.tap(); x = 200f }) { Text("Right") }
            Button(onClick = { BenchMarkers.tap(); y = -120f }) { Text("Up") }
            Button(onClick = { BenchMarkers.tap(); y = 120f }) { Text("Down") }
        }
    }
}

@Composable
private fun CombinedSection() {
    var grown by remember { mutableStateOf(false) }
    var pulsing by remember { mutableStateOf(false) }
    val scale by animateFloatAsState(
        if (grown) 1.6f else 1f,
        animationSpec = spring(dampingRatio = 0.4f),
        label = "c-scale",
    )
    val rotation by animateFloatAsState(
        if (grown) 180f else 0f,
        animationSpec = tween(600),
        label = "c-rot",
    )
    Column(Modifier.testTag("anim:combined")) {
        HeadlineText("Combined Transform")
        Box_(Modifier.graphicsLayer {
            scaleX = scale; scaleY = scale; rotationZ = rotation
        })
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { BenchMarkers.tap(); grown = false; pulsing = false }) { Text("Reset") }
            Button(onClick = { BenchMarkers.tap(); grown = true }) { Text("Grow + Spin") }
            Button(onClick = { BenchMarkers.tap(); pulsing = !pulsing }) { Text("Pulse") }
        }
        if (pulsing) {
            val transition = rememberInfiniteTransition(label = "pulse")
            val p by transition.animateFloat(
                0.8f, 1.2f,
                animationSpec = infiniteRepeatable(tween(500), repeatMode = androidx.compose.animation.core.RepeatMode.Reverse),
                label = "p",
            )
            Box_(
                Modifier.graphicsLayer { alpha = 2f - p; scaleX = p; scaleY = p },
                MaterialTheme.colorScheme.tertiary,
            )
        }
    }
}

@Composable
private fun ProgressSection() {
    var progress by remember { mutableFloatStateOf(0.5f) }
    val animated by animateFloatAsState(progress, tween(500), label = "progress")
    Column(Modifier.testTag("anim:progress")) {
        HeadlineText("Progress")
        LinearProgressIndicator(
            progress = { animated },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for ((label, v) in listOf("0%" to 0f, "25%" to 0.25f, "50%" to 0.5f, "75%" to 0.75f, "100%" to 1f)) {
                Button(onClick = { BenchMarkers.tap(); progress = v }) { Text(label) }
            }
        }
    }
}

@Composable
private fun SpringSection() {
    var on by remember { mutableStateOf(false) }
    val value by animateFloatAsState(
        if (on) 1f else 0f,
        animationSpec = spring(dampingRatio = 0.3f, stiffness = Spring.StiffnessLow),
        label = "spring",
    )
    Column(Modifier.testTag("anim:spring")) {
        HeadlineText("Spring")
        FootnoteText("value: ${"%.2f".format(value)}")
        Box_(Modifier.graphicsLayer { alpha = value; translationX = value * 300f })
        Button(onClick = {
            BenchMarkers.tap()
            on = !on
        }, modifier = Modifier.testTag("anim:spring-toggle")) { Text("Toggle") }
    }
}

@Composable
private fun CurvesSection() {
    var target by remember { mutableFloatStateOf(0.7f) }
    Column(Modifier.testTag("anim:curves")) {
        HeadlineText("Animation Curves")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for ((label, v) in listOf("Small (0.3)" to 0.3f, "Medium (0.7)" to 0.7f, "Large (1.0)" to 1f)) {
                Button(onClick = { BenchMarkers.tap(); target = v }) { Text(label) }
            }
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val specs = listOf(
                "linear" to tween<Float>(800, easing = LinearEasing),
                "ease" to tween<Float>(800, easing = FastOutSlowInEasing),
                "spring" to spring<Float>(dampingRatio = 0.35f),
            )
            for ((name, spec) in specs) {
                val h by animateFloatAsState(target, spec, label = name)
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Box(
                        Modifier.size(24.dp, (h * 120).dp)
                            .clip(RoundedCornerShape(4.dp))
                            .background(MaterialTheme.colorScheme.primary),
                    )
                    FootnoteText(name)
                }
            }
        }
    }
}

@Composable
private fun StaggeredSection() {
    val transition = rememberInfiniteTransition(label = "stagger")
    Column(Modifier.testTag("anim:stagger")) {
        HeadlineText("Staggered")
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            repeat(8) { i ->
                val v by transition.animateFloat(
                    0f, 1f,
                    animationSpec = infiniteRepeatable(
                        tween(1600, delayMillis = i * 120, easing = FastOutSlowInEasing),
                        repeatMode = androidx.compose.animation.core.RepeatMode.Reverse,
                    ),
                    label = "s$i",
                )
                Box(
                    Modifier.size(28.dp)
                        .graphicsLayer { translationY = -v * 30f }
                        .clip(RoundedCornerShape(6.dp))
                        .background(
                            MaterialTheme.colorScheme.primary.copy(
                                alpha = 0.3f + v * 0.7f,
                            ),
                        ),
                )
            }
        }
    }
}
