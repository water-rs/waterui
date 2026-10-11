package dev.waterui.android.reference

import android.Manifest
import android.content.pm.PackageManager
import android.hardware.camera2.CameraManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat

private val FILTER_NAMES = listOf("Natural", "Cinematic", "Noir", "Vintage", "Neon", "Dream")

/** The fixture's filter_params table: (brightness, saturation, contrast, tint, vignette). */
private fun filterParams(kind: Int, strength: Float): FloatArray = when (kind) {
    1 -> floatArrayOf(-0.05f * strength, 1.1f + 0.25f * strength, 1.0f + 0.3f * strength, 0.35f * strength, 0.45f * strength)
    2 -> floatArrayOf(-0.12f * strength, 1.0f - 0.95f * strength, 1.15f + 0.4f * strength, 0f, 0.2f * strength)
    3 -> floatArrayOf(0.02f * strength, 1.0f - 0.3f * strength, 1.0f + 0.1f * strength, 0.65f * strength, 0.35f * strength)
    4 -> floatArrayOf(0.08f * strength, 1.3f + 0.55f * strength, 1.15f + 0.45f * strength, -0.55f * strength, 0.15f * strength)
    5 -> floatArrayOf(0.1f * strength, 1.05f + 0.2f * strength, 0.92f + 0.08f * strength, 0.2f * strength, 0.6f * strength)
    else -> floatArrayOf(0f, 1f, 1f, 0f, 0f)
}

/**
 * Compose twin of the waterkit_camera_filters fixture: the synthetic camera
 * preview (the fixture's own GPU surface computes this clear color from the
 * filter params — reproduced exactly so both sides measure the same pixels),
 * the six presets, the strength slider, Reconnect, and the camera-inventory
 * bridge driven by CameraManager enumeration.
 */
@Composable
fun WaterkitCameraFiltersTwin() {
    val context = LocalContext.current
    var activeFilter by remember { mutableIntStateOf(0) }
    var strength by remember { mutableFloatStateOf(0.7f) }
    var previewStatus by remember { mutableStateOf("Synthetic camera frame.") }
    var bridgeStatus by remember { mutableStateOf("Not synced. Tap the button below to query Waterkit.") }
    var permissionStatus by remember { mutableStateOf("Permission status: unknown") }
    var inventory by remember { mutableStateOf("No camera inventory yet.") }
    var reconnectTicket by remember { mutableIntStateOf(0) }

    fun enumerate() {
        bridgeStatus = "Permission granted. Enumerating cameras via waterkit-camera..."
        val granted = ContextCompat.checkSelfPermission(
            context, Manifest.permission.CAMERA,
        ) == PackageManager.PERMISSION_GRANTED
        if (!granted) {
            bridgeStatus = "Camera permission was not granted. Sync cancelled."
            inventory = "Waterkit camera inventory unavailable until permission is granted."
            return
        }
        runCatching {
            val manager = context.getSystemService(CameraManager::class.java)
            val ids = manager.cameraIdList
            if (ids.isEmpty()) {
                bridgeStatus = "Waterkit is connected, but no camera devices were reported."
                inventory = "0 camera devices detected."
            } else {
                val summary = ids.joinToString(" | ") { id ->
                    val facing = when (
                        manager.getCameraCharacteristics(id)
                            .get(android.hardware.camera2.CameraCharacteristics.LENS_FACING)
                    ) {
                        android.hardware.camera2.CameraCharacteristics.LENS_FACING_FRONT -> "Front"
                        else -> "Back/External"
                    }
                    "$facing: camera ($id)"
                }
                bridgeStatus = "Waterkit sync complete: ${ids.size} camera(s) detected."
                inventory = summary
            }
        }.onFailure {
            bridgeStatus = "Camera enumeration failed: ${it.message}"
            inventory = "Waterkit camera list could not be loaded on this platform/runtime."
        }
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        permissionStatus = "Permission status: ${if (granted) "granted" else "denied"}"
        if (granted) enumerate()
        else {
            bridgeStatus = "Camera permission was not granted. Sync cancelled."
            inventory = "Waterkit camera inventory unavailable until permission is granted."
        }
    }

    val (brightness, saturation, contrast, tint, vignette) = filterParams(activeFilter, strength)
    val previewColor = Color(
        red = (0.32f + brightness + tint * 0.08f).coerceIn(0f, 1f),
        green = (0.46f + brightness + saturation * 0.04f - vignette * 0.03f).coerceIn(0f, 1f),
        blue = (0.58f + brightness - tint * 0.08f + contrast * 0.03f).coerceIn(0f, 1f),
    )

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        TitleText("WaterUI + Waterkit Camera Filter Lab")
        BodyText("Live camera preview via waterkit-camera, rendered and filtered with WaterUI GpuSurface.")
        HorizontalDivider()

        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            BodyText("Filter: ${FILTER_NAMES[activeFilter]}   |   Strength: ${"%.2f".format(strength)}")
            Box(
                Modifier.fillMaxWidth().aspectRatio(16f / 9f)
                    .clip(RoundedCornerShape(8.dp))
                    .background(previewColor)
                    .testTag("camera:preview"),
            )
            CaptionText(previewStatus)
            Button(
                onClick = {
                    BenchMarkers.tap()
                    reconnectTicket += 1
                    previewStatus = "Reconnecting camera stream..."
                },
                modifier = Modifier.testTag("camera:reconnect"),
            ) { Text("Reconnect Camera Stream") }
        }

        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            HeadlineText("Filter Presets")
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FILTER_NAMES.forEachIndexed { i, name ->
                    Surface(
                        onClick = {
                            BenchMarkers.tap()
                            activeFilter = i
                        },
                        shape = RoundedCornerShape(50),
                        color = if (i == activeFilter) {
                            MaterialTheme.colorScheme.primaryContainer
                        } else {
                            MaterialTheme.colorScheme.surfaceContainerHigh
                        },
                        modifier = Modifier.testTag("camera:preset-$name"),
                    ) {
                        Text(name, Modifier.padding(horizontal = 12.dp, vertical = 6.dp))
                    }
                }
            }
            Slider(
                value = strength,
                onValueChange = { strength = it },
                valueRange = 0f..1f,
                modifier = Modifier.testTag("camera:strength"),
            )
        }

        HorizontalDivider()
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            HeadlineText("Waterkit Bridge")
            BodyText(bridgeStatus)
            BodyText(permissionStatus)
            FootnoteText(inventory)
            Button(
                onClick = {
                    BenchMarkers.tap()
                    val granted = ContextCompat.checkSelfPermission(
                        context, Manifest.permission.CAMERA,
                    ) == PackageManager.PERMISSION_GRANTED
                    if (granted) {
                        permissionStatus = "Permission status: granted"
                        enumerate()
                    } else {
                        bridgeStatus = "Requesting camera permission via waterkit-permission..."
                        permissionLauncher.launch(Manifest.permission.CAMERA)
                    }
                },
                modifier = Modifier.testTag("camera:sync"),
            ) { Text("Sync with Waterkit Camera") }
        }
    }
}
