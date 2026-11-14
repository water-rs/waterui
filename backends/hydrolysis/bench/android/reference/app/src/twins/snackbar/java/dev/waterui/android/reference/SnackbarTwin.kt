package dev.waterui.android.reference

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.SnackbarVisuals
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Delete
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

private class IconVisuals(
    override val message: String,
    override val actionLabel: String? = null,
    val icon: ImageVector? = null,
    override val duration: SnackbarDuration = SnackbarDuration.Short,
    override val withDismissAction: Boolean = false,
) : SnackbarVisuals

/**
 * Compose twin of the snackbar fixture: the same seven presentations —
 * simple, icon, action+5s, top position, queued triple, independent
 * top+bottom banners, and a closeable snackbar that stays until dismissed.
 */
@Composable
fun SnackbarTwin() {
    val bottomHost = remember { SnackbarHostState() }
    val topHost = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()

    fun show(
        host: SnackbarHostState,
        message: String,
        icon: ImageVector? = null,
        action: String? = null,
        duration: SnackbarDuration = SnackbarDuration.Short,
        closeable: Boolean = false,
    ) {
        BenchMarkers.tap()
        scope.launch {
            val r = host.showSnackbar(IconVisuals(message, action, icon, duration, closeable))
            if (r == SnackbarResult.ActionPerformed) Unit
        }
    }

    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .padding(WATERUI_PADDING.dp),
            verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
        ) {
            TitleText("Snackbar Demo")
            Spacer(Modifier.weight(1f))
            Button(
                onClick = { show(bottomHost, "Hello from Snackbar!") },
                modifier = Modifier.testTag("snackbar:simple"),
            ) { Text("Simple Snackbar") }
            Button(
                onClick = { show(bottomHost, "File saved successfully", icon = Icons.Filled.CheckCircle) },
                modifier = Modifier.testTag("snackbar:icon"),
            ) { Text("With Icon") }
            Button(
                onClick = {
                    show(
                        bottomHost, "Item moved to trash", icon = Icons.Filled.Delete,
                        action = "Undo", duration = SnackbarDuration.Long,
                    )
                },
                modifier = Modifier.testTag("snackbar:action"),
            ) { Text("With Action Button") }
            Button(
                onClick = { show(topHost, "Network connected", icon = Icons.Filled.CheckCircle) },
                modifier = Modifier.testTag("snackbar:top"),
            ) { Text("Top Position") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    scope.launch {
                        bottomHost.showSnackbar("First message")
                        bottomHost.showSnackbar("Second message")
                        bottomHost.showSnackbar("Third message")
                    }
                },
                modifier = Modifier.testTag("snackbar:queue"),
            ) { Text("Queue Multiple") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    scope.launch {
                        // Independent placements coexist.
                        topHost.showSnackbar(
                            IconVisuals("Top banner", icon = Icons.Filled.CheckCircle),
                        )
                    }
                    scope.launch { bottomHost.showSnackbar("Bottom banner") }
                },
                modifier = Modifier.testTag("snackbar:top-bottom"),
            ) { Text("Top + Bottom") }
            Button(
                onClick = {
                    show(
                        bottomHost, "Stays until you close it",
                        duration = SnackbarDuration.Indefinite, closeable = true,
                    )
                },
                modifier = Modifier.testTag("snackbar:closeable"),
            ) { Text("Closeable") }
            Spacer(Modifier.weight(1f))
        }

        SnackbarHost(
            hostState = topHost,
            modifier = Modifier.align(Alignment.TopCenter).testTag("snackbar-host-top"),
        ) { data -> IconSnackbar(data) }
        SnackbarHost(
            hostState = bottomHost,
            modifier = Modifier.align(Alignment.BottomCenter).testTag("snackbar-host-bottom"),
        ) { data -> IconSnackbar(data) }
    }
}

@Composable
private fun IconSnackbar(data: androidx.compose.material3.SnackbarData) {
    val visuals = data.visuals as? IconVisuals
    Snackbar(
        action = data.visuals.actionLabel?.let { label ->
            { TextButton(onClick = { data.performAction() }) { Text(label) } }
        },
        dismissAction = if (visuals?.withDismissAction == true || (visuals != null && visuals.duration == SnackbarDuration.Indefinite && visuals.actionLabel == null)) {
            { TextButton(onClick = { data.dismiss() }) { Text("Close") } }
        } else {
            null
        },
    ) {
        androidx.compose.foundation.layout.Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            visuals?.icon?.let { Icon(it, contentDescription = null) }
            Text(data.visuals.message)
        }
    }
}
