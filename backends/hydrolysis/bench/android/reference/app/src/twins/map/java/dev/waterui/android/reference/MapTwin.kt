package dev.waterui.android.reference

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/**
 * Compose twin of the map fixture: a pinned local Manhattan tile rendered at
 * the fixture's camera (40.7580, -73.9855), the icon-only zoom in/out
 * controls, Use My Location and the "Manhattan" status panel. The bundled
 * tile stands in for network tile fetch per the pinned-assets rule.
 */
@Composable
fun MapTwin() {
    val context = LocalContext.current
    val tile = remember {
        context.assets.open("tile_manhattan.png").use {
            BitmapFactory.decodeStream(it)
        }.asImageBitmap()
    }
    var zoom by remember { mutableFloatStateOf(14f) }
    var locating by remember { mutableStateOf(false) }
    var statusClicks by remember { mutableIntStateOf(0) }

    Box(Modifier.fillMaxSize()) {
        // Map surface: the pinned tile, scaled/centered by the camera.
        Box(
            Modifier.fillMaxSize().clip(RoundedCornerShape(0.dp))
                .testTag("map:surface"),
        ) {
            Image(
                bitmap = tile,
                contentDescription = "Manhattan map",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop,
            )
        }

        // Status panel (top).
        Column(
            Modifier.fillMaxWidth()
                .padding(12.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                .padding(12.dp)
                .testTag("map:status"),
        ) {
            TitleText("Manhattan")
            CaptionText(
                if (locating) {
                    "Locating…"
                } else {
                    "40.7580, -73.9855 · zoom ${"%.1f".format(zoom)} · panned $statusClicks"
                },
            )
        }

        // Controls (bottom-trailing): locate + zoom in/out.
        Column(
            Modifier.align(Alignment.BottomEnd).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(
                onClick = {
                    BenchMarkers.tap()
                    locating = true
                },
                modifier = Modifier.testTag("map:locate"),
            ) { Text("Use My Location") }
            Column(
                Modifier.clip(RoundedCornerShape(12.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh),
            ) {
                Button(onClick = {
                    BenchMarkers.tap()
                    zoom = (zoom + 0.030f).coerceAtMost(20f)
                }, modifier = Modifier.testTag("map:zoom-in")) { Text("+") }
                Button(onClick = {
                    BenchMarkers.tap()
                    zoom = (zoom - 0.030f).coerceAtLeast(1f)
                }, modifier = Modifier.testTag("map:zoom-out")) { Text("−") }
            }
        }
    }
}
