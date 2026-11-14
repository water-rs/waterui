package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp

// Twin of examples/hover: hover/cursor demo sections. Initial state only —
// nothing is hovered or dragged, so counters read zero and both zstack colour
// layers resolve to their inactive opacity.

private val HoverActive = Color(0xFF4CAF50)
private val HoverInactive = Color(0xFF2196F3)
private val DragActive = Color(0xFFFF5722)
private val DragInactive = Color(0xFFFF9800)

private val CursorColors = listOf(
    "Arrow" to 0xFF9E9E9E,
    "Hand" to 0xFF2196F3,
    "Text" to 0xFF4CAF50,
    "Cross" to 0xFFFF9800,
    "Grab" to 0xFF9C27B0,
    "Grabbing" to 0xFF673AB7,
    "No" to 0xFFF44336,
    "Wait" to 0xFF795548,
    "H-Resize" to 0xFF00BCD4,
    "V-Resize" to 0xFF009688,
    "Move" to 0xFF607D8B,
    "Copy" to 0xFF8BC34A,
)

/** `zstack` of two colour layers under padded text — active layer at 0 alpha. */
@Composable
private fun HoverSurface(
    label: String,
    inactive: Color,
    active: Color,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier, contentAlignment = Alignment.Center) {
        Box(Modifier.matchParentSize().background(inactive.copy(alpha = 0.3f)))
        Box(Modifier.matchParentSize().background(active.copy(alpha = 0f)))
        BodyText(label, modifier = Modifier.padding(WATERUI_PADDING.dp))
    }
}

@Composable
private fun CursorBox(name: String, color: Color) {
    Box(
        modifier = Modifier
            .width(96.dp)
            .height(44.dp)
            .background(color.copy(alpha = 0.3f)),
        contentAlignment = Alignment.Center,
    ) {
        CaptionText(name)
    }
}

@Composable
private fun HoverSection(
    title: String,
    caption: String,
    content: @Composable () -> Unit,
) {
    VStack(modifier = Modifier.padding(WATERUI_PADDING.dp)) {
        HeadlineText(title)
        BodyText(caption)
        content()
    }
}

@Composable
fun HoverTwin() {
    Column(
        modifier = Modifier
            .verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp)
            .fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        VStack {
            TitleText("WaterUI Hover & Cursor Examples")
            BodyText("Demonstrating hover events, cursor styles, and lifecycle hooks")
            HorizontalDivider()
            Spacer(Modifier.height(0.dp))

            HoverSection("Hover Events", "Move your pointer in and out of the box") {
                HStack {
                    BodyText("Hover events: ")
                    BodyText("Count: 0")
                }
                HStack {
                    BodyText("Currently hovered: ")
                    BodyText("Status: false")
                }
                HoverSurface("Hover Me!", HoverInactive, HoverActive,
                    Modifier.width(200.dp).height(80.dp))
            }
            HorizontalDivider()

            HoverSection("Cursor Styles", "Hover over each box to see different cursor styles") {
                HStack { (0..3).forEach { CursorBox(CursorColors[it].first, Color(CursorColors[it].second)) } }
                HStack { (4..7).forEach { CursorBox(CursorColors[it].first, Color(CursorColors[it].second)) } }
                HStack { (8..11).forEach { CursorBox(CursorColors[it].first, Color(CursorColors[it].second)) } }
            }
            HorizontalDivider()

            HoverSection("Reactive Cursor", "The cursor changes based on drag state") {
                HStack {
                    BodyText("State: ")
                    BodyText("Dragging: false")
                }
                BodyText("(Hover to simulate drag state change)")
                HoverSurface("Drag Area", DragInactive, DragActive,
                    Modifier.width(200.dp).height(100.dp))
            }
            HorizontalDivider()

            HoverSection("Interactive Buttons", "Buttons naturally have cursor changes") {
                HStack {
                    OutlinedButton(onClick = {}) { Text("Bordered") }
                    TextButton(onClick = {}) { Text("Plain") }
                    TextButton(onClick = {}) {
                        Text(
                            "Link Style",
                            color = MaterialTheme.colorScheme.primary,
                            textDecoration = TextDecoration.Underline,
                        )
                    }
                }
                BodyText("Link buttons show pointer cursor by default")
            }
            Spacer(Modifier.height(0.dp))
        }
    }
}
