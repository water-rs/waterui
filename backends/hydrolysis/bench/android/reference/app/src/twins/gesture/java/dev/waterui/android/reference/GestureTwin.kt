package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

// Twin of examples/gesture: six gesture sections over a scrollable stack.
// Initial state only — all counters read zero, the chained gesture shows its
// waiting prompt, because only the settled first screen is compared.

private val TapColor = Color(0xFF2196F3)
private val DoubleTapColor = Color(0xFF4CAF50)
private val LongPressColor = Color(0xFFFF9800)
private val DragColor = Color(0xFF9C27B0)
private val ChainedColor = Color(0xFFF44336)
private val OnTapColor = Color(0xFF00BCD4)

/** `text(label).padding().background(color.with_opacity(0.3))` */
@Composable
private fun GestureBox(
    label: String,
    color: Color,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier.background(color.copy(alpha = 0.3f)),
        contentAlignment = Alignment.Center,
    ) {
        BodyText(label, modifier = Modifier.padding(WATERUI_PADDING.dp))
    }
}

@Composable
private fun GestureSection(
    title: String,
    caption: String,
    counter: String,
    box: @Composable () -> Unit,
) {
    VStack(modifier = Modifier.padding(WATERUI_PADDING.dp)) {
        HeadlineText(title)
        BodyText(caption)
        BodyText(counter)
        box()
    }
}

@Composable
fun GestureTwin() {
    Column(
        modifier = Modifier
            .verticalScroll(rememberScrollState())
            .padding(16.dp)
            .fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        VStack {
            TitleText("WaterUI Gesture Examples")
            BodyText("Demonstrating gesture recognition and handling")
            HorizontalDivider()
            Spacer(Modifier.height(0.dp))

            GestureSection(
                title = "Tap Gesture",
                caption = "Tap the box below to increment the counter",
                counter = "Tap count: 0",
            ) { GestureBox("Tap Me!", TapColor) }
            HorizontalDivider()

            GestureSection(
                title = "Double Tap Gesture",
                caption = "Double-tap the box to increment",
                counter = "Double tap count: 0",
            ) { GestureBox("Double Tap Me!", DoubleTapColor) }
            HorizontalDivider()

            GestureSection(
                title = "Long Press Gesture",
                caption = "Press and hold for 500ms",
                counter = "Long press count: 0",
            ) { GestureBox("Long Press Me!", LongPressColor) }
            HorizontalDivider()

            GestureSection(
                title = "Drag Gesture",
                caption = "Drag within the box (min 5pt)",
                counter = "Drag events: 0",
            ) {
                GestureBox(
                    "Drag Here",
                    DragColor,
                    Modifier.width(200.dp).height(100.dp),
                )
            }
            HorizontalDivider()

            GestureSection(
                title = "Chained Gesture",
                caption = "Tap first, then long press to complete",
                counter = "Waiting for tap...",
            ) { GestureBox("Tap then Long Press", ChainedColor) }
            HorizontalDivider()

            GestureSection(
                title = "on_tap Shorthand",
                caption = "Convenient method for simple tap handlers",
                counter = "This uses the same counter as Section 1",
            ) { GestureBox("Simple Tap", OnTapColor) }
        }
    }
}
