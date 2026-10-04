package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * Compose twin of the edge_text fixture: pathological static text —
 * unbroken runs, combining marks, ZWJ emoji, mixed scripts, empty strings
 * and size extremes.
 */
@Composable
fun EdgeTextTwin() {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Edge Text")
        CaptionText("Static text edge cases: the same strings the fixture "
            + "pushes through the WaterUI text pipeline.")

        HeadlineText("Unbroken 160-char run")
        Text(
            "A".repeat(160),
            modifier = Modifier.fillMaxWidth().testTag("edge:unbroken"),
            style = MaterialTheme.typography.bodyMedium,
        )

        HeadlineText("Combining marks")
        Text(
            "a\u0301e\u0301i\u0301o\u0301u\u0301 — comb\u0334in\u0334ing strikethrough",
            modifier = Modifier.testTag("edge:combining"),
            style = MaterialTheme.typography.bodyLarge,
        )

        HeadlineText("ZWJ emoji")
        Text(
            "👨\u200D👩\u200D👧\u200D👦 👩\u200D💻 🏳️\u200D🌈 ❤️\u200D🔥",
            modifier = Modifier.testTag("edge:zwj"),
            style = MaterialTheme.typography.headlineSmall,
        )

        HeadlineText("Mixed scripts")
        Text(
            "English العربية 日本語 中文 Русский 한국어 mixed on one line",
            modifier = Modifier.testTag("edge:scripts"),
            style = MaterialTheme.typography.bodyLarge,
        )

        HeadlineText("Empty and whitespace")
        Text("", modifier = Modifier.testTag("edge:empty"))
        Text("   ", modifier = Modifier.testTag("edge:whitespace"))
        FootnoteText("(empty and whitespace-only runs render above)")

        HeadlineText("Size extremes")
        Text(
            "nine point",
            modifier = Modifier.testTag("edge:tiny"),
            fontSize = 9.sp,
        )
        Text(
            "forty-eight",
            modifier = Modifier.testTag("edge:huge"),
            fontSize = 48.sp,
            fontWeight = FontWeight.Bold,
        )

        HeadlineText("Wrapping paragraph")
        Text(
            "The quick brown fox jumps over the lazy dog. ".repeat(12).trim(),
            modifier = Modifier.fillMaxWidth().testTag("edge:paragraph"),
            style = MaterialTheme.typography.bodyMedium,
        )
    }
}
