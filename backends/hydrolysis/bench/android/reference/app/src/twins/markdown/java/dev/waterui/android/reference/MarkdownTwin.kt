package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

private fun matchOffsets(needle: String, document: String): List<Int> {
    if (needle.isEmpty()) return emptyList()
    val out = mutableListOf<Int>()
    var i = document.indexOf(needle, ignoreCase = true)
    while (i >= 0) {
        out += i
        i = document.indexOf(needle, i + 1, ignoreCase = true)
    }
    return out
}

private fun excerpt(offset: Int, document: String): String {
    val start = offset - 40
    val begin = if (start < 0) 0 else (document.indexOf(' ', start) + 1)
    val end = document.indexOf(' ', offset + 40).let { if (it < 0) document.length else it }
    return "…${document.substring(begin, end)}…"
}

/**
 * Compose twin of the markdown fixture: the bundled example.md rendered by
 * the shared renderer under a search overlay (Find button, query field,
 * n-of-m status, excerpt preview, submit advances, Escape/Done close).
 */
@Composable
fun MarkdownTwin() {
    val context = LocalContext.current
    val document = remember {
        context.assets.open("example.md").bufferedReader().use { it.readText() }
    }
    var open by remember { mutableStateOf(false) }
    var query by remember { mutableStateOf("") }
    var current by remember { mutableIntStateOf(0) }
    val focusRequester = remember { FocusRequester() }

    val matches = matchOffsets(query, document)
    val status = if (matches.isEmpty()) "No matches" else "${current % matches.size + 1} of ${matches.size}"
    val preview = matches.getOrNull(current % matches.size.coerceAtLeast(1))
        ?.let { excerpt(it, document) } ?: ""

    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .padding(WATERUI_PADDING.dp)
                .testTag("markdown:doc"),
        ) {
            MarkdownText(document)
        }

        Column(Modifier.fillMaxSize().padding(WATERUI_PADDING.dp)) {
            Row {
                Button(
                    onClick = {
                        BenchMarkers.tap()
                        open = true
                    },
                    modifier = Modifier.testTag("markdown:find"),
                ) { Text("Find") }
                Spacer(Modifier.weight(1f))
            }
            if (open) {
                Surface(
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    modifier = Modifier.fillMaxWidth()
                        .testTag("markdown:search-bar")
                        .onKeyEvent { event ->
                            if (event.key == Key.Escape) {
                                open = false
                                true
                            } else {
                                false
                            }
                        },
                ) {
                    Column(Modifier.padding(WATERUI_PADDING.dp)) {
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
                        ) {
                            OutlinedTextField(
                                value = query,
                                onValueChange = {
                                    BenchMarkers.edit()
                                    query = it
                                    current = 0
                                },
                                modifier = Modifier.weight(1f)
                                    .focusRequester(focusRequester)
                                    .testTag("markdown:search-field"),
                                singleLine = true,
                                placeholder = { Text("Search") },
                                keyboardActions = KeyboardActions(
                                    onDone = {
                                        if (matches.isNotEmpty()) {
                                            current = (current + 1) % matches.size
                                        }
                                    },
                                ),
                            )
                            CaptionText(status)
                            Button(
                                onClick = {
                                    BenchMarkers.tap()
                                    open = false
                                },
                                modifier = Modifier.testTag("markdown:done"),
                            ) { Text("Done") }
                        }
                        if (preview.isNotEmpty()) CaptionText(preview)
                    }
                }
            }
            Spacer(Modifier.weight(1f))
        }
    }
}
