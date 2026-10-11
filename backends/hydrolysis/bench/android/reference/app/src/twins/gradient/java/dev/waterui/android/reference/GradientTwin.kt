package dev.waterui.android.reference

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.sin

private val MESH = listOf(
    Color(0xFFEF4444), Color(0xFFF59E0B), Color(0xFF10B981),
    Color(0xFF3B82F6), Color(0xFF8B5CF6), Color(0xFFEC4899),
)

/**
 * Compose twin of the gradient fixture: animated background, animated mesh,
 * shape fill, linear, radial, static mesh and HDR-range gradient sections —
 * the same visual families the fixture exercises.
 */
@Composable
fun GradientTwin() {
    val transition = rememberInfiniteTransition(label = "gradient")
    val phase by transition.animateFloat(
        initialValue = 0f, targetValue = 1f,
        animationSpec = infiniteRepeatable(
            tween(4000, easing = LinearEasing), RepeatMode.Reverse,
        ),
        label = "phase",
    )

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Gradients")

        HeadlineText("Animated Background")
        Box(
            Modifier.fillMaxWidth().height(140.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(
                    Brush.linearGradient(
                        listOf(MESH[0], MESH[3], MESH[1]),
                        start = Offset(0f, 0f),
                        end = Offset(600f * phase + 200f, 140f),
                    ),
                )
                .testTag("gradient:animated"),
        ) { CaptionText("animated linear sweep") }

        HeadlineText("Animated Mesh")
        Box(
            Modifier.fillMaxWidth().height(140.dp)
                .clip(RoundedCornerShape(12.dp))
                .drawBehind {
                    val w = size.width
                    val h = size.height
                    MESH.forEachIndexed { i, color ->
                        val t = phase * 2 * PI.toFloat() + i
                        val cx = w * (0.5f + 0.4f * sin(t + i))
                        val cy = h * (0.5f + 0.4f * sin(t * 1.3f + i * 2))
                        drawCircle(
                            brush = Brush.radialGradient(
                                listOf(color.copy(alpha = 0.85f), Color.Transparent),
                                center = Offset(cx, cy),
                                radius = w * 0.5f,
                            ),
                        )
                    }
                }
                .testTag("gradient:mesh-animated"),
        ) { CaptionText("GPU-animated mesh counterpart") }

        HeadlineText("Shape Fill")
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier.size(72.dp).clip(CircleShape)
                    .background(Brush.linearGradient(listOf(MESH[1], MESH[4])))
                    .testTag("gradient:shape-circle"),
            )
            Box(
                Modifier.size(96.dp, 72.dp).clip(RoundedCornerShape(16.dp))
                    .background(Brush.radialGradient(listOf(MESH[2], MESH[3])))
                    .testTag("gradient:shape-rounded"),
            )
        }

        HeadlineText("Linear")
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier.weight(1f).height(80.dp).clip(RoundedCornerShape(12.dp))
                    .background(
                        Brush.horizontalGradient(listOf(MESH[0], MESH[1], MESH[2])),
                    )
                    .testTag("gradient:linear-h"),
            )
            Box(
                Modifier.weight(1f).height(80.dp).clip(RoundedCornerShape(12.dp))
                    .background(
                        Brush.verticalGradient(listOf(MESH[3], MESH[5])),
                    )
                    .testTag("gradient:linear-v"),
            )
        }

        HeadlineText("Radial")
        Box(
            Modifier.fillMaxWidth().height(120.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(
                    Brush.radialGradient(
                        listOf(Color.White, MESH[3], MESH[5]),
                        radius = 400f,
                    ),
                )
                .testTag("gradient:radial"),
        )

        HeadlineText("Mesh")
        Box(
            Modifier.fillMaxWidth().height(140.dp)
                .clip(RoundedCornerShape(12.dp))
                .drawBehind {
                    val w = size.width
                    val h = size.height
                    val cols = 3
                    MESH.forEachIndexed { i, color ->
                        val cx = w * ((i % cols) + 0.5f) / cols
                        val cy = h * ((i / cols) + 0.5f) / (MESH.size / cols)
                        drawCircle(
                            brush = Brush.radialGradient(
                                listOf(color.copy(alpha = 0.9f), Color.Transparent),
                                center = Offset(cx, cy),
                                radius = w / cols,
                            ),
                        )
                    }
                }
                .testTag("gradient:mesh"),
        )

        HeadlineText("HDR Range")
        Box(
            Modifier.fillMaxWidth().height(80.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(
                    Brush.horizontalGradient(
                        listOf(
                            Color(0xFFFF3300), Color(0xFFFFFF00),
                            Color(0xFFFFFFFF), Color(0xFF00EEFF),
                        ),
                    ),
                )
                .testTag("gradient:hdr"),
        ) { CaptionText("wide-gamut-style ramp") }
    }
}
