package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DragHandle
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.toMutableStateList
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import my.nanihadesuka.compose.LazyColumnScrollbar
import my.nanihadesuka.compose.ScrollbarSelectionMode
import my.nanihadesuka.compose.ScrollbarSettings
import sh.calvin.reorderable.ReorderableItem
import sh.calvin.reorderable.rememberReorderableLazyListState
import kotlinx.coroutines.launch
import java.text.NumberFormat

// Twin of examples/list: stable records, lazy rows, animated jumps and editing.
private const val DATASET_SIZE = 100_000
// hydrolysis-m3 ButtonSmallTokens leading/trailing space and vertical content padding.
private val listButtonPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)

@Composable
fun ListTwin() {
    val records = remember { (0 until DATASET_SIZE).toList().toMutableStateList() }
    val listState = rememberLazyListState()
    val reorderState = rememberReorderableLazyListState(listState) { from, to ->
        records.add(to.index, records.removeAt(from.index))
    }
    val scope = rememberCoroutineScope()
    var editing by remember { mutableStateOf(false) }
    Column(
        modifier = Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        Column(
            horizontalAlignment = Alignment.Start,
            verticalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.padding(WATERUI_PADDING.dp),
        ) {
            TitleText("100,000-row lazy List")
            SubheadlineText("${NumberFormat.getIntegerInstance().format(records.size)} active rows", color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedButton(contentPadding = listButtonPadding, onClick = {
                    scope.launch { listState.animateScrollToItem(0) }
                }) { Text("Top") }
                OutlinedButton(contentPadding = listButtonPadding, onClick = {
                    scope.launch { listState.animateScrollToItem(records.size / 2) }
                }) { Text("Middle") }
                OutlinedButton(contentPadding = listButtonPadding, onClick = {
                    if (records.isNotEmpty()) {
                        scope.launch { listState.animateScrollToItem(records.lastIndex) }
                    }
                }) { Text("Last") }
                Button(contentPadding = listButtonPadding, onClick = { editing = !editing }) { Text(if (editing) "Done" else "Edit") }
            }
            CaptionText(
                "Animated jumps and the draggable scrollbar keep only viewport rows materialized.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        HorizontalDivider()
        BoxWithConstraints(Modifier.weight(1f)) {
            LazyColumnScrollbar(
                state = listState,
                modifier = Modifier.fillMaxSize(),
                settings = ScrollbarSettings(
                    alwaysShowScrollbar = true,
                    selectionMode = ScrollbarSelectionMode.Full,
                    // Hydrolysis Material3 scroll-indicator tokens.
                    thumbThickness = 2.5.dp,
                    scrollbarPadding = 1.5.dp,
                    thumbMinLength = 12.dp / maxHeight,
                    thumbUnselectedColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.55f),
                    thumbSelectedColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.55f),
                ),
            ) {
                LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
                    itemsIndexed(records, key = { _, record -> record }) { index, record ->
                        ReorderableItem(reorderState, key = record, enabled = editing) {
                            // hydrolysis-m3 LIST_SWIPE_DISMISS_THRESHOLD is half the row.
                            val dismissState = rememberSwipeToDismissBoxState(positionalThreshold = { it * 0.5f })
                            SwipeToDismissBox(
                                state = dismissState,
                                modifier = Modifier.semantics {
                                    customActions = listOf(CustomAccessibilityAction("Delete") {
                                        records.remove(record)
                                        true
                                    })
                                },
                                onDismiss = { records.remove(record) },
                                backgroundContent = {
                                    Box(
                                        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.error).padding(16.dp),
                                        contentAlignment = if (dismissState.dismissDirection == SwipeToDismissBoxValue.StartToEnd) {
                                            Alignment.CenterStart
                                        } else {
                                            Alignment.CenterEnd
                                        },
                                    ) {
                                        Icon(Icons.Default.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.onError)
                                    }
                                },
                            ) {
                                Box(Modifier.background(MaterialTheme.colorScheme.surface)) {
                                    // Material ListRowMetrics insets surround the example content.
                                    Row(
                                        verticalAlignment = Alignment.CenterVertically,
                                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
                                    ) {
                                        Column(
                                            horizontalAlignment = Alignment.Start,
                                            verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
                                            modifier = Modifier.weight(1f).padding(vertical = 10.dp, horizontal = 16.dp),
                                        ) {
                                            SubheadlineText(String.format("Record #%,06d", record))
                                            CaptionText(
                                                "Materialized only while this row is visible",
                                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            )
                                        }
                                        if (editing) {
                                            IconButton(onClick = { records.remove(record) }) {
                                                Icon(
                                                    Icons.Default.Delete,
                                                    contentDescription = "Delete",
                                                    tint = MaterialTheme.colorScheme.error,
                                                )
                                            }
                                            Icon(
                                                Icons.Default.DragHandle,
                                                contentDescription = "Move Record #%,06d".format(record),
                                                modifier = Modifier.size(48.dp).draggableHandle().semantics {
                                                    customActions = buildList {
                                                        if (index > 0) add(CustomAccessibilityAction("Move up") {
                                                            records.add(index - 1, records.removeAt(index))
                                                            true
                                                        })
                                                        if (index < records.lastIndex) add(CustomAccessibilityAction("Move down") {
                                                            records.add(index + 1, records.removeAt(index))
                                                            true
                                                        })
                                                    }
                                                }.padding(12.dp),
                                            )
                                        }
                                    }
                                    HorizontalDivider(
                                        modifier = Modifier.align(Alignment.BottomCenter).padding(horizontal = 16.dp),
                                        color = MaterialTheme.colorScheme.outline,
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
