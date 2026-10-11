package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

private val EllipseShape = GenericShape { size, _ ->
    addOval(androidx.compose.ui.geometry.Rect(0f, 0f, size.width, size.height))
}
private val CapsuleShape = RoundedCornerShape(percent = 50)

private fun polygonShape(points: Int, innerRatio: Float = 1f) = GenericShape { size, _ ->
    val cx = size.width / 2f
    val cy = size.height / 2f
    val r = size.minDimension / 2f
    val steps = if (innerRatio < 1f) points * 2 else points
    for (i in 0 until steps) {
        val rr = if (innerRatio < 1f && i % 2 == 1) r * innerRatio else r
        val a = -PI / 2 + i * 2 * PI / steps
        val x = cx + rr * cos(a).toFloat()
        val y = cy + rr * sin(a).toFloat()
        if (i == 0) moveTo(x, y) else lineTo(x, y)
    }
    close()
}

/**
 * Compose twin of the shape fixture: built-in shapes, custom star and
 * hexagon paths, a corner-radius morph slider and clipped content.
 */
@Composable
fun ShapeTwin() {
    var morph by remember { mutableFloatStateOf(0f) }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Shapes")
        CaptionText("Built-in and custom path shapes, morphing and clipping.")

        HeadlineText("Built-ins")
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier.size(64.dp).clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primary)
                    .testTag("shape:circle"),
            )
            Box(
                Modifier.size(96.dp, 64.dp).clip(EllipseShape)
                    .background(MaterialTheme.colorScheme.secondary)
                    .testTag("shape:ellipse"),
            )
            Box(
                Modifier.size(96.dp, 48.dp).clip(CapsuleShape)
                    .background(MaterialTheme.colorScheme.tertiary)
                    .testTag("shape:capsule"),
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier.size(96.dp, 56.dp).clip(RoundedCornerShape(0.dp))
                    .background(MaterialTheme.colorScheme.primaryContainer)
                    .testTag("shape:rect"),
            )
            Box(
                Modifier.size(96.dp, 56.dp).clip(RoundedCornerShape(16.dp))
                    .background(MaterialTheme.colorScheme.secondaryContainer)
                    .testTag("shape:rounded"),
            )
            Box(
                Modifier.size(96.dp, 56.dp)
                    .clip(
                        androidx.compose.foundation.shape.RoundedCornerShape(
                            topStart = 24.dp, topEnd = 4.dp,
                            bottomStart = 4.dp, bottomEnd = 24.dp,
                        ),
                    )
                    .background(MaterialTheme.colorScheme.tertiaryContainer)
                    .testTag("shape:uneven"),
            )
        }

        HeadlineText("Custom paths")
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier.size(72.dp).clip(polygonShape(5, innerRatio = 0.45f))
                    .background(MaterialTheme.colorScheme.primary)
                    .testTag("shape:star"),
            )
            Box(
                Modifier.size(72.dp).clip(polygonShape(6))
                    .background(MaterialTheme.colorScheme.secondary)
                    .testTag("shape:hexagon"),
            )
        }

        HeadlineText("Corner morph")
        BodyText("Radius: ${(morph * 50).toInt()}%")
        Slider(
            value = morph,
            onValueChange = { morph = it },
            modifier = Modifier.fillMaxWidth().testTag("shape:morph-slider"),
        )
        Box(
            Modifier.fillMaxWidth().height(80.dp)
                .clip(RoundedCornerShape((morph * 40).dp))
                .background(MaterialTheme.colorScheme.primary)
                .testTag("shape:morph"),
        )

        HeadlineText("Clipped content")
        Box(
            Modifier.fillMaxWidth().height(80.dp)
                .clip(polygonShape(6))
                .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                .border(2.dp, MaterialTheme.colorScheme.outline, polygonShape(6))
                .testTag("shape:clip"),
        ) {
            BodyText("Content clipped to a hexagon path")
        }
    }
}
