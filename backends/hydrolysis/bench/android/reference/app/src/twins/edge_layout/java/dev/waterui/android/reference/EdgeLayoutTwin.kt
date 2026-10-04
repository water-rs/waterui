package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/**
 * Compose twin of the edge_layout fixture: layout stress cases — deep
 * nesting, an eager 16x10 grid and edge-to-edge constraint demos.
 */
@Composable
fun EdgeLayoutTwin() {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Edge Layout")
        CaptionText("Deep nesting, a 16×10 eager grid and edge-constrained content.")

        HeadlineText("Deep nesting (8 levels)")
        DeepNest(0)

        HeadlineText("16 × 10 eager grid")
        Column(
            Modifier.fillMaxWidth().testTag("edge:grid"),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            repeat(10) { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                    repeat(16) { col ->
                        val hue = (row * 16 + col) * 360f / 160f
                        Box(
                            Modifier.weight(1f).aspectRatio(1f)
                                .clip(RoundedCornerShape(2.dp))
                                .background(
                                    Color.hsv(hue, 0.55f, 0.9f),
                                ),
                        )
                    }
                }
            }
        }

        HeadlineText("Constraint edges")
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Box(
                Modifier.fillMaxWidth().height(48.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(MaterialTheme.colorScheme.primaryContainer)
                    .testTag("edge:full-width"),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Box(
                    Modifier.weight(1f).height(48.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(MaterialTheme.colorScheme.secondaryContainer),
                )
                Box(
                    Modifier.weight(1f).height(48.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(MaterialTheme.colorScheme.tertiaryContainer),
                )
            }
        }
    }
}

@Composable
private fun DeepNest(depth: Int) {
    if (depth >= 8) {
        Box(
            Modifier.size(24.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(MaterialTheme.colorScheme.primary)
                .testTag("edge:nest-$depth"),
        )
        return
    }
    Row(
        Modifier.padding(2.dp)
            .clip(RoundedCornerShape(4.dp))
            .background(
                MaterialTheme.colorScheme.surfaceContainerHigh.copy(
                    alpha = 0.4f + depth * 0.07f,
                ),
            )
            .padding(2.dp),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        DeepNest(depth + 1)
    }
}
