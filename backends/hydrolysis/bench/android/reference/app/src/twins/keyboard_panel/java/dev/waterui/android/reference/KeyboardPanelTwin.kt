package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

private val SelfBubbleColor = Color(0xFF0B93F6)
private val PeerBubbleColor = Color(0xFFE9E9EB)
private val ComposerFillColor = Color(0xFFF7F7F9)

private val ConversationMessages =
    listOf(
        false to "Are we still on for Saturday?",
        true to "Yes — trailhead at nine, right?",
        false to "Nine works. The weather is supposed to be clear.",
        true to "I'll bring coffee and the topo map.",
        false to "Can you grab me an extra water bottle?",
        true to "Done. Two litres in the side pocket.",
        false to "Perfect. See you there.",
        true to "See you. Don't forget the permits this time.",
    )

@Composable
fun KeyboardPanelTwin() {
    var draft by remember { mutableStateOf("") }
    Column(Modifier.fillMaxSize()) {
        Column(
            Modifier.weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            ConversationMessages.forEach { (isSelf, message) ->
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = if (isSelf) Arrangement.End else Arrangement.Start,
                ) {
                    Text(
                        text = message,
                        color = if (isSelf) Color.White else Color.Black,
                        modifier =
                            Modifier.background(if (isSelf) SelfBubbleColor else PeerBubbleColor)
                                .padding(10.dp),
                    )
                }
            }
        }
        Row(
            Modifier.fillMaxWidth()
                .background(ComposerFillColor)
                .navigationBarsPadding()
                .imePadding()
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = draft,
                onValueChange = { draft = it },
                placeholder = { Text("Message") },
                modifier = Modifier.weight(1f),
                singleLine = true,
            )
            Spacer(Modifier.width(8.dp))
            Button(
                onClick = {
                    BenchMarkers.tap()
                    draft = ""
                },
            ) {
                Text("Send")
            }
        }
    }
}
