package dev.waterui.android.reference

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.draganddrop.dragAndDropSource
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.DragAndDropTransferData
import androidx.compose.ui.draganddrop.mimeTypes
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

private data class Fruit(val emoji: String, val label: String, val color: Color)

private val FRUITS = listOf(
    Fruit("🍎", "Apple", Color(0xFFEF4444)),
    Fruit("🍊", "Orange", Color(0xFFF97316)),
    Fruit("🍋", "Lemon", Color(0xFFEAB308)),
    Fruit("🍇", "Grape", Color(0xFF8B5CF6)),
    Fruit("🍓", "Strawberry", Color(0xFFEC4899)),
    Fruit("🥝", "Kiwi", Color(0xFF22C55E)),
)

private const val FRUIT_MIME = "application/x.waterui-fruit"

/**
 * Compose twin of the drag_drop fixture: draggable fruit cards, a drop
 * basket with hover highlight and a bounce on drop, the collected list and
 * Clear Basket.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun DragDropTwin() {
    var collected by remember { mutableStateOf(listOf<Fruit>()) }
    var hovering by remember { mutableStateOf(false) }
    val bounce = remember { Animatable(1f) }
    val scope = rememberCoroutineScope()

    val dropTarget = remember {
        object : DragAndDropTarget {
            override fun onStarted(event: DragAndDropEvent) {
                hovering = true
            }

            override fun onEnded(event: DragAndDropEvent) {
                hovering = false
            }

            override fun onDrop(event: DragAndDropEvent): Boolean {
                hovering = false
                val clip = event.toAndroidDragEvent().clipData
                val label = if (clip.itemCount > 0) clip.getItemAt(0).text?.toString() else null
                val fruit = FRUITS.firstOrNull { it.label == label } ?: return false
                collected = collected + fruit
                scope.launch {
                    bounce.snapTo(0.9f)
                    bounce.animateTo(1f, spring(dampingRatio = 0.4f))
                }
                return true
            }
        }
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Drag fruits into the basket!")

        HeadlineText("Drag these fruits")
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (row in FRUITS.chunked(2)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (fruit in row) {
                        FruitCard(fruit, Modifier.weight(1f))
                    }
                }
            }
        }

        Box(
            Modifier.fillMaxWidth().height(140.dp)
                .scale(bounce.value)
                .clip(RoundedCornerShape(16.dp))
                .background(
                    if (hovering) {
                        MaterialTheme.colorScheme.primaryContainer
                    } else {
                        MaterialTheme.colorScheme.surfaceContainerHigh
                    },
                )
                .border(
                    width = if (hovering) 3.dp else 1.dp,
                    color = if (hovering) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.outline
                    },
                    shape = RoundedCornerShape(16.dp),
                )
                .dragAndDropTarget(
                    shouldStartDragAndDrop = { event ->
                        event.mimeTypes().contains(FRUIT_MIME) ||
                            event.mimeTypes().contains("text/plain")
                    },
                    target = dropTarget,
                )
                .testTag("basket"),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    collected.joinToString(" ") { it.emoji },
                    fontSize = 40.sp,
                )
                Text(
                    if (collected.isEmpty()) {
                        "Drop fruits here!"
                    } else {
                        "${collected.size} fruit${if (collected.size == 1) "" else "s"} collected!"
                    },
                    fontSize = 14.sp,
                )
            }
        }

        if (collected.isNotEmpty()) {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                for (fruit in collected) {
                    BodyText("${fruit.emoji} ${fruit.label}")
                }
            }
        }

        Button(
            onClick = {
                BenchMarkers.tap()
                collected = emptyList()
            },
            modifier = Modifier.testTag("basket:clear"),
        ) { Text("Clear Basket") }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FruitCard(fruit: Fruit, modifier: Modifier = Modifier) {
    Row(
        modifier
            .clip(RoundedCornerShape(12.dp))
            .background(fruit.color.copy(alpha = 0.18f))
            .border(1.dp, fruit.color, RoundedCornerShape(12.dp))
            .dragAndDropSource(block = {
                DragAndDropTransferData(
                    clipData = android.content.ClipData.newPlainText(
                        "fruit", fruit.label,
                    ),
                    localState = fruit,
                )
            })
            .padding(horizontal = 12.dp, vertical = 8.dp)
            .testTag("fruit:${fruit.label}"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(fruit.emoji, fontSize = 28.sp)
        Text(fruit.label, fontSize = 16.sp, fontWeight = FontWeight.Medium)
    }
}
