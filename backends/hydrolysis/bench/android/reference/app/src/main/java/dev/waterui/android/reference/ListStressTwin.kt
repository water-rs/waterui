package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

private const val STRESS_ROWS = 1_000

private data class StressItem(
    val id: Int,
    val title: String,
    val detail: String,
    val colorIndex: Int,
)

/**
 * The deterministic 1,000-row list stress case from section 6 — a
 * suite-only screen (not a frozen fixture), hosted through twinFor.
 * Row content is a pure function of the index, so the WaterUI and Compose
 * sides enumerate byte-identical data.
 */
@Composable
fun ListStressTwin() {
    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().padding(WATERUI_PADDING.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
        ) {
            HeadlineText("1,000-row stress")
            CaptionText("deterministic")
        }
        LazyColumn(Modifier.fillMaxSize().testTag("stress:list")) {
            items(
                count = STRESS_ROWS,
                key = { it },
                contentType = { "row" },
            ) { index ->
                val line2 = index % 2 == 0
                Row(
                    Modifier.fillMaxWidth()
                        .padding(horizontal = WATERUI_PADDING.dp, vertical = 6.dp)
                        .testTag("stress-row-$index"),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Column(Modifier.weight(1f)) {
                        BodyText("Item $index")
                        FootnoteText(
                            "Subtitle for item $index" + if (line2) " — second line" else "",
                        )
                    }
                    Text(
                        "#${index % 97}",
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
