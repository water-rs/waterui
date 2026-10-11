package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties

private data class WindowSpec(
    val title: String,
    val caption: String,
    val colored: Color? = null,
    val frosted: Boolean = false,
    val transparent: Boolean = false,
)

private val WINDOWS = listOf(
    WindowSpec("Standard Titled Window", "Classic window with title bar and opaque background"),
    WindowSpec("Borderless Window", "Frameless window with colored semi-transparent background", colored = Color(0x993B82F6)),
    WindowSpec("Frosted Glass Window", "Window with material blur effect (Regular thickness)", frosted = true),
    WindowSpec("Transparent Overlay", "Fully transparent window with FullSizeContentView style", transparent = true),
    WindowSpec("Ultra-Thin Material Window", "Subtle frosted effect with UltraThin material", frosted = true, colored = Color(0x44FFFFFF)),
)

/**
 * Compose twin of the multi_window fixture: five window-style sections, each
 * with Open/Close — on Android each window presents as a Dialog carrying the
 * section's title and style treatment.
 */
@Composable
fun MultiWindowTwin() {
    var openIndex by remember { mutableStateOf(-1) }

    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .padding(WATERUI_PADDING.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            TitleText("Multi-Window Gallery")
            BodyText("Explore different window styles and backgrounds")
            HorizontalDivider()
            WINDOWS.forEachIndexed { i, spec ->
                Column(
                    Modifier.fillMaxWidth().testTag("window:${spec.title.lowercase().replace(' ', '-')}"),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    HeadlineText(spec.title)
                    FootnoteText(spec.caption)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(
                            onClick = {
                                BenchMarkers.tap()
                                openIndex = i
                            },
                            modifier = Modifier.testTag("window:open-$i"),
                        ) { Text("Open Window") }
                        Button(
                            onClick = {
                                BenchMarkers.tap()
                                openIndex = -1
                            },
                            modifier = Modifier.testTag("window:close-$i"),
                        ) { Text("Close Window") }
                    }
                }
                if (i < WINDOWS.lastIndex) HorizontalDivider()
            }
        }

        if (openIndex in WINDOWS.indices) {
            val spec = WINDOWS[openIndex]
            Dialog(
                onDismissRequest = { openIndex = -1 },
                properties = DialogProperties(
                    usePlatformDefaultWidth = spec.transparent,
                ),
            ) {
                Card(
                    Modifier.width(320.dp),
                    shape = RoundedCornerShape(if (spec.title.startsWith("Borderless")) 0.dp else 16.dp),
                    colors = CardDefaults.cardColors(
                        containerColor = when {
                            spec.transparent -> Color(0x66FFFFFF)
                            spec.colored != null -> spec.colored
                            else -> MaterialTheme.colorScheme.surfaceContainerHigh
                        },
                    ),
                ) {
                    Column(
                        Modifier.fillMaxWidth().padding(20.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        TitleText(spec.title)
                        BodyText(spec.caption)
                        if (spec.frosted) {
                            Box(
                                Modifier.fillMaxWidth().height(60.dp)
                                    .clip(RoundedCornerShape(8.dp))
                                    .background(
                                        MaterialTheme.colorScheme.surface.copy(alpha = 0.4f),
                                    )
                                    .blur(4.dp),
                            ) { FootnoteText("material blur region") }
                        }
                        Button(
                            onClick = {
                                BenchMarkers.tap()
                                openIndex = -1
                            },
                            modifier = Modifier.testTag("window:dialog-close"),
                        ) { Text("Close") }
                    }
                }
            }
        }
    }
}
