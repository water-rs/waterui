package dev.waterui.android.reference

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

private const val STREAM_CPS_MIN = 4
private const val STREAM_CPS_MAX = 128
private val PRESETS = listOf("assistant", "minimal", "none")
private val DOCS = listOf(
    "Ops digest" to "llm_ops_digest.md",
    "Release brief" to "llm_release_brief.md",
    "Incident report" to "llm_incident_report.md",
)

/**
 * Compose twin of the flow_markdown fixture: three bundled LLM documents
 * streaming through the shared markdown renderer at a configurable CPS, with
 * the fixture's full control set (Prev/Next doc, Start stream, Load full,
 * Reset, LLM CPS ±, preset cycle, reveal CPS ±, token-fade toggle).
 */
@Composable
fun FlowMarkdownTwin() {
    val context = LocalContext.current
    val docs = remember {
        DOCS.map { (title, file) ->
            title to context.assets.open(file).bufferedReader().use { it.readText() }
        }
    }
    val scope = rememberCoroutineScope()

    var docIndex by remember { mutableIntStateOf(0) }
    var charProgress by remember { mutableIntStateOf(0) }
    var streaming by remember { mutableStateOf(false) }
    var streamJob by remember { mutableStateOf<Job?>(null) }
    var streamCps by remember { mutableIntStateOf(32) }
    var animCps by remember { mutableIntStateOf(64) }
    var preset by remember { mutableIntStateOf(0) }
    var tokenFade by remember { mutableStateOf(true) }

    fun cancel() {
        streamJob?.cancel()
        streaming = false
    }

    fun reset() {
        cancel()
        charProgress = 0
    }

    fun start() {
        cancel()
        streaming = true
        streamJob = scope.launch {
            val body = docs[docIndex].second
            val intervalMs = (1000L / streamCps.coerceIn(STREAM_CPS_MIN, STREAM_CPS_MAX))
                .coerceIn(8, 40)
            while (isActive && charProgress < body.length) {
                delay(intervalMs)
                charProgress = (charProgress + 1).coerceAtMost(body.length)
            }
            streaming = charProgress < body.length
        }
    }

    Column(
        Modifier.fillMaxSize().padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        TitleText("Flow Markdown")
        SubheadlineText("Document: ${docs[docIndex].first} (${docIndex + 1}/${docs.size})")
        CaptionText(
            "LLM output progress: $charProgress/${docs[docIndex].second.length} chars",
        )
        CaptionText(
            "LLM stream speed: $streamCps chars/s | stream: ${if (streaming) "running" else "idle"}",
        )

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = {
                    BenchMarkers.tap()
                    docIndex = (docIndex - 1 + docs.size) % docs.size
                    reset()
                },
                modifier = Modifier.width(140.dp).testTag("flow:prev"),
            ) { Text("Prev doc") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    docIndex = (docIndex + 1) % docs.size
                    reset()
                },
                modifier = Modifier.width(140.dp).testTag("flow:next"),
            ) { Text("Next doc") }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = {
                    BenchMarkers.tap()
                    start()
                },
                modifier = Modifier.width(140.dp).testTag("flow:start"),
                colors = ButtonDefaults.filledTonalButtonColors(),
            ) { Text("Start stream") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    cancel()
                    charProgress = docs[docIndex].second.length
                },
                modifier = Modifier.width(140.dp).testTag("flow:load-full"),
            ) { Text("Load full") }
        }

        Button(
            onClick = {
                BenchMarkers.tap()
                reset()
            },
            modifier = Modifier.width(140.dp).testTag("flow:reset"),
        ) { Text("Reset") }

        CaptionText(
            "Flow animation preset: ${PRESETS[preset]} | token reveal CPS: $animCps | " +
                "token fade: ${if (tokenFade) "on" else "off"}",
        )

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = {
                    BenchMarkers.tap()
                    streamCps = (streamCps - 4).coerceIn(STREAM_CPS_MIN, STREAM_CPS_MAX)
                },
                modifier = Modifier.testTag("flow:llm-cps-minus"),
            ) { Text("LLM CPS -") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    streamCps = (streamCps + 4).coerceIn(STREAM_CPS_MIN, STREAM_CPS_MAX)
                },
                modifier = Modifier.testTag("flow:llm-cps-plus"),
            ) { Text("LLM CPS +") }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = {
                    BenchMarkers.tap()
                    preset = (preset + 1) % 3
                },
                modifier = Modifier.testTag("flow:preset"),
            ) { Text("Preset") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    animCps = (animCps - 8).coerceIn(8, 256)
                },
                modifier = Modifier.testTag("flow:cps-minus"),
            ) { Text("CPS -") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    animCps = (animCps + 8).coerceIn(8, 256)
                },
                modifier = Modifier.testTag("flow:cps-plus"),
            ) { Text("CPS +") }
        }

        Button(
            onClick = {
                BenchMarkers.tap()
                tokenFade = !tokenFade
            },
            modifier = Modifier.testTag("flow:token-fade"),
        ) { Text(if (tokenFade) "Token fade on" else "Token fade off") }

        HorizontalDivider()

        Column(
            Modifier.fillMaxWidth()
                .border(1.dp, MaterialTheme.colorScheme.outline)
                .verticalScroll(rememberScrollState())
                .padding(8.dp)
                .testTag("flow:content"),
        ) {
            MarkdownText(docs[docIndex].second.take(charProgress))
        }
    }
}
