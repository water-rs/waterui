package dev.waterui.android.reference

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

// Twin of examples/list: header controls over a 100,000-row lazy list.
// Initial state: 100000 rows, not editing, scrolled to top.
//
// Button translations: bordered -> OutlinedButton, borderedProminent -> filled
// Button; the backend's list rows are outlined MaterialCardViews, so the twin
// uses OutlinedCard.

private const val DATASET_SIZE = 100_000

@Composable
fun ListTwin() {
    Column(modifier = Modifier.fillMaxWidth()) {
        Column(
            horizontalAlignment = Alignment.Start,
            modifier = Modifier
                .padding(WATERUI_PADDING.dp)
                .fillMaxWidth(),
        ) {
            TitleText("100,000-row lazy List")
            SubheadlineText("100,000 active rows")
            HStack {
                OutlinedButton(onClick = {}) { Text("Top") }
                OutlinedButton(onClick = {}) { Text("Middle") }
                OutlinedButton(onClick = {}) { Text("Last") }
                Button(onClick = {}) { Text("Edit") }
            }
            CaptionText(
                "Animated jumps and the draggable scrollbar keep only viewport rows materialized."
            )
        }
        HorizontalDivider()
        LazyColumn {
            items(DATASET_SIZE) { index ->
                OutlinedCard(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp)
                ) {
                    Column(
                        horizontalAlignment = Alignment.Start,
                        modifier = Modifier.padding(vertical = 10.dp, horizontal = 16.dp),
                    ) {
                        SubheadlineText(String.format("Record #%06d", index))
                        CaptionText("Materialized only while this row is visible")
                    }
                }
            }
        }
    }
}
