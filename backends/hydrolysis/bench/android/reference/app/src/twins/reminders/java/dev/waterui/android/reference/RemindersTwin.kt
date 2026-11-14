package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material.icons.filled.Flag
import androidx.compose.material.icons.filled.Inbox
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

private data class ReminderRow(
    val id: Int,
    val title: String,
    val subtitle: String? = null,
    val flagged: Boolean = false,
)

private enum class Destination(
    val label: String,
    val color: Color,
    val selectedColor: Color,
    val count: Int,
    val icon: ImageVector,
) {
    Today("Today", Color(0xFF4A84F6), Color(0xFF3A6FD4), 6, Icons.Filled.DateRange),
    Scheduled("Scheduled", Color(0xFFF2483F), Color(0xFFD43C34), 2, Icons.Filled.DateRange),
    All("All", Color(0xFF54545A), Color(0xFF44444A), 18, Icons.Filled.Inbox),
    Flagged("Flagged", Color(0xFFF28A34), Color(0xFFD97A2B), 1, Icons.Filled.Flag),
    Urgent("Urgent", Color(0xFFE0517E), Color(0xFFC9446E), 0, Icons.Filled.Notifications),
    Completed("Completed", Color(0xFF8E8E93), Color(0xFF76767B), 12, Icons.Filled.Check),
}

private val USER_LISTS = listOf(
    Triple("Reminders", 37, Color(0xFFF28A34)),
    Triple("Best Shot", 2, Color(0xFF4A84F6)),
    Triple("Road map of water", 12, Color(0xFF4A84F6)),
)

private fun remindersFor(dest: Destination): Pair<List<ReminderRow>, List<ReminderRow>> = when (dest) {
    Destination.Today -> listOf(
        ReminderRow(1, "Call dentist", "2:00 PM"),
        ReminderRow(2, "Review navigation parity worktree", "Before lunch", flagged = true),
    ) to listOf(
        ReminderRow(3, "Pick up package", "Tomorrow 10:00 AM"),
    )
    Destination.Scheduled -> listOf(
        ReminderRow(4, "Book flight", "Fri"),
    ) to listOf(
        ReminderRow(5, "Pay utilities", "Next week"),
    )
    Destination.All -> listOf(
        ReminderRow(6, "Plan weekend"),
        ReminderRow(7, "Update roadmap", flagged = true),
    ) to listOf(
        ReminderRow(8, "Refactor split navigation chrome", "Cross-platform"),
    )
    Destination.Flagged -> listOf(
        ReminderRow(9, "Prepare demo", "High priority", flagged = true),
    ) to emptyList()
    Destination.Urgent -> emptyList<ReminderRow>() to emptyList()
    Destination.Completed -> listOf(
        ReminderRow(10, "Submit timesheet"),
        ReminderRow(11, "Clean inbox"),
    ) to emptyList()
}

/**
 * Compose twin of the reminders fixture: the six smart-list tiles in the
 * official palette, the My Lists rows, a search field, and the per-list
 * reminder detail with check-off circles and flagged markers.
 */
@Composable
fun RemindersTwin() {
    var selection by remember { mutableStateOf(Destination.Today) }
    var search by remember { mutableStateOf("") }
    var completed by remember { mutableStateOf(setOf<Int>()) }

    val query = search.trim().lowercase().ifEmpty { null }
    val (active, done) = remindersFor(selection)
    val filtered = (active + done).filter { row ->
        query == null || row.title.lowercase().contains(query) ||
            row.subtitle?.lowercase()?.contains(query) == true
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        // Smart-list tile grid (2 columns × 3 rows).
        Destination.entries.toList().chunked(2).forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (dest in row) {
                    Column(
                        Modifier.weight(1f)
                            .clip(RoundedCornerShape(12.dp))
                            .background(
                                if (selection == dest) dest.selectedColor else dest.color,
                            )
                            .clickable {
                                BenchMarkers.tap()
                                selection = dest
                            }
                            .padding(12.dp)
                            .testTag("reminders:tile-${dest.name.lowercase()}"),
                    ) {
                        Icon(
                            dest.icon,
                            contentDescription = dest.label,
                            tint = Color.White,
                            modifier = Modifier.size(20.dp),
                        )
                        Spacer(Modifier.height(18.dp))
                        Text(
                            "${dest.count}",
                            color = Color.White,
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.Bold,
                        )
                        Text(
                            dest.label,
                            color = Color.White.copy(alpha = 0.85f),
                            style = MaterialTheme.typography.labelSmall,
                        )
                    }
                }
            }
        }

        HeadlineText("My Lists")
        for ((name, count, color) in USER_LISTS) {
            Row(
                Modifier.fillMaxWidth().padding(vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Box(
                    Modifier.size(28.dp).clip(CircleShape).background(color),
                )
                BodyText(name)
                Spacer(Modifier.weight(1f))
                CaptionText("$count")
            }
        }

        OutlinedTextField(
            value = search,
            onValueChange = {
                BenchMarkers.edit()
                search = it
            },
            modifier = Modifier.fillMaxWidth().testTag("reminders:search"),
            singleLine = true,
            placeholder = { Text("Search") },
        )

        HeadlineText(selection.label)
        if (filtered.isEmpty()) {
            CaptionText("No reminders")
        }
        for (row in filtered) {
            Row(
                Modifier.fillMaxWidth()
                    .clickable {
                        BenchMarkers.tap()
                        completed = if (row.id in completed) completed - row.id else completed + row.id
                    }
                    .padding(vertical = 8.dp)
                    .testTag("reminders:row-${row.id}"),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                RadioButton(
                    selected = row.id in completed || selection == Destination.Completed,
                    onClick = null,
                )
                Column(Modifier.weight(1f)) {
                    BodyText(row.title)
                    row.subtitle?.let { FootnoteText(it) }
                }
                if (row.flagged) {
                    Icon(
                        Icons.Filled.Flag, "flagged",
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(16.dp),
                    )
                }
            }
        }
    }
}
