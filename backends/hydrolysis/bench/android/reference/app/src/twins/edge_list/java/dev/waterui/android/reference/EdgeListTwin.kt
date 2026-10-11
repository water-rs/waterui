package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

private const val ROW_COUNT = 200_000

/**
 * Compose twin of the edge_list fixture: 200,000 variable-height rows —
 * 1-3 detail lines and a deterministic colored chip per row — through
 * LazyColumn virtualization.
 */
@Composable
fun EdgeListTwin() {
    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().padding(WATERUI_PADDING.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
        ) {
            HeadlineText("200,000 rows")
            CaptionText("variable height")
        }
        LazyColumn(Modifier.fillMaxSize().testTag("edge:list")) {
            items(
                count = ROW_COUNT,
                key = { it },
                contentType = { "row" },
            ) { index ->
                Row(
                    Modifier.fillMaxWidth()
                        .padding(horizontal = WATERUI_PADDING.dp, vertical = 6.dp)
                        .testTag("edge:row"),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Column(Modifier.weight(1f)) {
                        BodyText("Row $index")
                        val lines = index % 3 + 1
                        for (line in 1 until lines) {
                            FootnoteText("Detail line $line for row $index")
                        }
                    }
                    Text(
                        "chip $index",
                        modifier = Modifier
                            .clip(RoundedCornerShape(10.dp))
                            .background(
                                when (index % 4) {
                                    0 -> MaterialTheme.colorScheme.primaryContainer
                                    1 -> MaterialTheme.colorScheme.secondaryContainer
                                    2 -> MaterialTheme.colorScheme.tertiaryContainer
                                    else -> MaterialTheme.colorScheme.surfaceContainerHigh
                                },
                            )
                            .padding(horizontal = 8.dp, vertical = 2.dp),
                        style = MaterialTheme.typography.labelSmall,
                    )
                }
            }
        }
    }
}
