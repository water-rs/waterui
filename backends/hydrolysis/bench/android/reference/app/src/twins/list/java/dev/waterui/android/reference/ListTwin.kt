package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
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
import kotlinx.coroutines.launch
import java.text.NumberFormat

// Twin of examples/list: stable records, lazy rows, animated jumps and editing.
private const val DATASET_SIZE = 100_000

@Composable
fun ListTwin() {
    val records = remember { (0 until DATASET_SIZE).toList().toMutableStateList() }
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    var editing by remember { mutableStateOf(false) }
    Column(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        Column(
            horizontalAlignment = Alignment.Start,
            verticalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.padding(WATERUI_PADDING.dp).fillMaxWidth(),
        ) {
            TitleText("100,000-row lazy List")
            SubheadlineText("${NumberFormat.getIntegerInstance().format(records.size)} active rows", color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedButton(onClick = {
                    scope.launch { listState.animateScrollToItem(0) }
                }) { Text("Top") }
                OutlinedButton(onClick = {
                    scope.launch { listState.animateScrollToItem(records.size / 2) }
                }) { Text("Middle") }
                OutlinedButton(onClick = {
                    if (records.isNotEmpty()) {
                        scope.launch { listState.animateScrollToItem(records.lastIndex) }
                    }
                }) { Text("Last") }
                Button(onClick = { editing = !editing }) { Text(if (editing) "Done" else "Edit") }
            }
            CaptionText(
                "Animated jumps and the draggable scrollbar keep only viewport rows materialized.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        HorizontalDivider()
        LazyColumn(state = listState, modifier = Modifier.weight(1f)) {
            itemsIndexed(records, key = { _, record -> record }) { index, record ->
                Box(modifier = Modifier.animateItem()) {
                    // Material ListRowMetrics adds these insets around the example content.
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
                    ) {
                        if (editing) {
                            IconButton(onClick = { records.removeAt(index) }) {
                                Icon(Icons.Default.Delete, contentDescription = "Delete")
                            }
                        }
                        Column(
                            horizontalAlignment = Alignment.Start,
                            verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
                            modifier = Modifier.weight(1f).padding(vertical = 10.dp, horizontal = 16.dp),
                        ) {
                            SubheadlineText(String.format("Record #%06d", record))
                            CaptionText(
                                "Materialized only while this row is visible",
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (editing) {
                            Column {
                                IconButton(
                                    enabled = index > 0,
                                    onClick = { records.add(index - 1, records.removeAt(index)) },
                                ) { Icon(Icons.Default.KeyboardArrowUp, contentDescription = "Move up") }
                                IconButton(
                                    enabled = index < records.lastIndex,
                                    onClick = { records.add(index + 1, records.removeAt(index)) },
                                ) { Icon(Icons.Default.KeyboardArrowDown, contentDescription = "Move down") }
                            }
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
