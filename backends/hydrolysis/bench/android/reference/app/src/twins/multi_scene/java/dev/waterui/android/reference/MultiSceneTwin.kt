package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

@Composable
fun MultiSceneTwin() {
    val hostState = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Multi-Scene",
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold,
            )
            Text("Every scene builds this content fresh.")
            Button(
                onClick = {
                    BenchMarkers.tap()
                    scope.launch {
                        hostState.showSnackbar(
                            message = "Only this scene's snackbar",
                            duration = SnackbarDuration.Indefinite,
                        )
                    }
                },
            ) {
                Text("Show snackbar")
            }
            Spacer(Modifier.weight(1f))
        }
        SnackbarHost(
            hostState = hostState,
            modifier = Modifier.align(Alignment.BottomCenter),
        )
    }
}
