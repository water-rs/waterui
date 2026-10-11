package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties

private enum class Edge { Top, Bottom, Leading, Trailing }
private enum class EdgeAlignment { Center, Start, End }

/**
 * Anchors a popup to the requested edge of its trigger with a 6px gap and a
 * window clamp, flipping to the opposite edge when the card does not fit —
 * the placement rules the fixture's AnchoredOverlay demos exercise.
 */
private class EdgePopupProvider(
    val edge: Edge,
    val alignment: EdgeAlignment = EdgeAlignment.Center,
    val gap: Int = 6,
    val margin: Int = 8,
) : PopupPositionProvider {
    override fun calculatePosition(
        anchorBounds: IntRect,
        windowSize: IntSize,
        layoutDirection: LayoutDirection,
        popupContentSize: IntSize,
    ): IntOffset {
        fun crossAxis(anchorStart: Int, anchorSize: Int, popupSize: Int) = when (alignment) {
            EdgeAlignment.Start -> anchorStart
            EdgeAlignment.End -> anchorStart + anchorSize - popupSize
            EdgeAlignment.Center -> anchorStart + (anchorSize - popupSize) / 2
        }

        fun place(e: Edge): IntOffset = when (e) {
            Edge.Top -> IntOffset(
                crossAxis(anchorBounds.left, anchorBounds.width, popupContentSize.width),
                anchorBounds.top - popupContentSize.height - gap,
            )
            Edge.Bottom -> IntOffset(
                crossAxis(anchorBounds.left, anchorBounds.width, popupContentSize.width),
                anchorBounds.bottom + gap,
            )
            Edge.Leading -> IntOffset(
                anchorBounds.left - popupContentSize.width - gap,
                crossAxis(anchorBounds.top, anchorBounds.height, popupContentSize.height),
            )
            Edge.Trailing -> IntOffset(
                anchorBounds.right + gap,
                crossAxis(anchorBounds.top, anchorBounds.height, popupContentSize.height),
            )
        }

        fun fits(p: IntOffset) =
            p.x >= margin && p.y >= margin &&
                p.x + popupContentSize.width <= windowSize.width - margin &&
                p.y + popupContentSize.height <= windowSize.height - margin

        val preferred = place(edge)
        if (fits(preferred)) return preferred
        val flipped = place(
            when (edge) {
                Edge.Top -> Edge.Bottom
                Edge.Bottom -> Edge.Top
                Edge.Leading -> Edge.Trailing
                Edge.Trailing -> Edge.Leading
            },
        )
        val p = if (fits(flipped)) flipped else preferred
        return IntOffset(
            p.x.coerceIn(margin, (windowSize.width - popupContentSize.width - margin).coerceAtLeast(margin)),
            p.y.coerceIn(margin, (windowSize.height - popupContentSize.height - margin).coerceAtLeast(margin)),
        )
    }
}

@Composable
private fun OverlayCard(note: String, tall: Boolean = false) {
    Box(
        Modifier.wrapContentSize()
            .heightIn(min = if (tall) 96.dp else 0.dp)
            .background(
                MaterialTheme.colorScheme.surfaceContainerHigh,
                RoundedCornerShape(8.dp),
            )
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
            .padding(12.dp),
    ) { CaptionText(note) }
}

@Composable
private fun AnchorButton(
    title: String,
    note: String,
    edge: Edge,
    tall: Boolean = false,
    alignment: EdgeAlignment = EdgeAlignment.Center,
    manual: Boolean = false,
) {
    var open by remember { mutableStateOf(false) }
    Button(
        onClick = {
            BenchMarkers.tap()
            open = !open
        },
        modifier = Modifier.testTag("anchor:${title.lowercase().replace(' ', '-').replace("(", "").replace(")", "")}"),
    ) {
        Text(title)
        if (open) {
            Popup(
                popupPositionProvider = EdgePopupProvider(edge, alignment),
                onDismissRequest = { if (!manual) open = false },
                properties = PopupProperties(focusable = manual),
            ) { OverlayCard(note, tall) }
        }
    }
}

/** Compose twin of the anchored_overlay fixture's three demo rows. */
@Composable
fun AnchoredOverlayTwin() {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        TitleText("Anchored Overlay")
        CaptionText("Tap an anchor; the overlay tracks it and stays inside the window.")

        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            AnchorButton("Top (flips)", "Top edge", Edge.Top, tall = true)
            AnchorButton("Top Start", "Top edge, start", Edge.Top, alignment = EdgeAlignment.Start)
            AnchorButton("Top End", "Top edge, end", Edge.Top, alignment = EdgeAlignment.End)
        }

        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            AnchorButton("Leading", "Leading edge", Edge.Leading)
            AnchorButton("Bottom", "Bottom edge", Edge.Bottom)
            AnchorButton("Trailing", "Trailing edge", Edge.Trailing)
            AnchorButton("Top (fits)", "Top edge", Edge.Top)
            AnchorButton("Manual", "Trailing edge, manual", Edge.Trailing, manual = true)
        }

        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            AnchorButton("Bottom (flips)", "Bottom edge", Edge.Bottom, tall = true)
            AnchorButton("Bottom Start", "Bottom edge, start", Edge.Bottom, alignment = EdgeAlignment.Start)
            AnchorButton("Bottom End", "Bottom edge, end", Edge.Bottom, alignment = EdgeAlignment.End)
        }
    }
}
