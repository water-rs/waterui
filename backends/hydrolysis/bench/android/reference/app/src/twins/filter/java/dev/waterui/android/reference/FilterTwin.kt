package dev.waterui.android.reference

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

private val SAMPLE_COLORS = listOf(
    Color.Red, Color.Green, Color.Blue,
    Color.Yellow, Color.Magenta, Color.Cyan,
)

@Composable
private fun SampleContent(modifier: Modifier = Modifier) {
    Column(modifier.heightIn(min = 100.dp)) {
        for (row in SAMPLE_COLORS.chunked(3)) {
            Row {
                for (c in row) {
                    Box(Modifier.size(40.dp).background(c))
                }
            }
        }
    }
}


private fun contrastMatrix(c: Float): ColorMatrix {
    val t = (1f - c) * 128f
    return ColorMatrix(
        floatArrayOf(
            c, 0f, 0f, 0f, t,
            0f, c, 0f, 0f, t,
            0f, 0f, c, 0f, t,
            0f, 0f, 0f, 1f, 0f,
        ),
    )
}


@Composable
private fun FilterSection(
    title: String,
    caption: String,
    value: Float,
    onValue: (Float) -> Unit,
    range: ClosedFloatingPointRange<Float>,
    presets: List<Pair<String, Float>>,
    tag: String,
    filtered: @Composable () -> Unit,
) {
    Column(
        Modifier.fillMaxWidth().testTag("filter:$tag"),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        HeadlineText(title)
        FootnoteText(caption)
        filtered()
        Slider(value = value, onValueChange = onValue, valueRange = range)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for ((label, v) in presets) {
                Button(onClick = {
                    BenchMarkers.tap()
                    onValue(v)
                }) { Text(label) }
            }
        }
    }
}

/**
 * Compose twin of the filter fixture: live blur/brightness/saturation/
 * contrast/hue/grayscale/opacity filters over the same six-swatch sample,
 * plus the combined-filter presets.
 */
@Composable
fun FilterTwin() {
    var blur by remember { mutableFloatStateOf(0f) }
    var brightness by remember { mutableFloatStateOf(0f) }
    var saturation by remember { mutableFloatStateOf(1f) }
    var contrast by remember { mutableFloatStateOf(1f) }
    var hue by remember { mutableFloatStateOf(0f) }
    var grayscale by remember { mutableFloatStateOf(0f) }
    var opacity by remember { mutableFloatStateOf(1f) }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Filters")

        FilterSection(
            "Blur", "Gaussian blur radius (0–20)",
            blur, { blur = it }, 0f..20f,
            listOf("0" to 0f, "5" to 5f, "10" to 10f, "20" to 20f),
            "blur",
        ) { SampleContent(Modifier.blur(blur.dp)) }

        FilterSection(
            "Brightness", "Adjust brightness (-1–1)",
            brightness, { brightness = it }, -1f..1f,
            listOf("-1" to -1f, "0" to 0f, "0.5" to 0.5f, "1" to 1f),
            "brightness",
        ) {
            val b = brightness
            SampleContent(
                Modifier.drawWithContent {
                    drawContent()
                    if (b > 0f) drawRect(Color.White.copy(alpha = b))
                    else if (b < 0f) drawRect(Color.Black.copy(alpha = -b))
                },
            )
        }

        FilterSection(
            "Saturation", "Color intensity (0–2)",
            saturation, { saturation = it }, 0f..2f,
            listOf("0" to 0f, "1" to 1f, "1.5" to 1.5f, "2" to 2f),
            "saturation",
        ) {
            SampleContent(
                Modifier.graphicsLayer {
                    colorFilter = ColorFilter.colorMatrix(saturationMatrix(saturation))
                },
            )
        }

        FilterSection(
            "Contrast", "Contrast (0–2)",
            contrast, { contrast = it }, 0f..2f,
            listOf("0" to 0f, "1" to 1f, "1.5" to 1.5f, "2" to 2f),
            "contrast",
        ) {
            SampleContent(
                Modifier.graphicsLayer {
                    colorFilter = ColorFilter.colorMatrix(contrastMatrix(contrast))
                },
            )
        }

        FilterSection(
            "Hue Rotation", "Rotate hue (0–360°)",
            hue, { hue = it }, 0f..360f,
            listOf("0" to 0f, "90" to 90f, "180" to 180f, "360" to 360f),
            "hue",
        ) {
            SampleContent(
                Modifier.graphicsLayer {
                    colorFilter = ColorFilter.colorMatrix(hueMatrix(hue))
                },
            )
        }

        FilterSection(
            "Grayscale", "Convert to grayscale (0 = color, 1 = grayscale)",
            grayscale, { grayscale = it }, 0f..1f,
            listOf("0" to 0f, "0.5" to 0.5f, "1" to 1f),
            "grayscale",
        ) {
            SampleContent(
                Modifier.graphicsLayer {
                    colorFilter = ColorFilter.colorMatrix(
                        saturationMatrix(1f - grayscale),
                    )
                },
            )
        }

        FilterSection(
            "Opacity", "Adjust transparency (0 = invisible, 1 = opaque)",
            opacity, { opacity = it }, 0f..1f,
            listOf("0" to 0f, "0.5" to 0.5f, "1" to 1f),
            "opacity",
        ) { SampleContent(Modifier.alpha(opacity)) }

        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            HeadlineText("Combined Filters")
            FootnoteText("Apply multiple filters with spring animations")
            SampleContent(
                Modifier
                    .blur(blur.dp.coerceAtMost(20.dp))
                    .graphicsLayer {
                        colorFilter = ColorFilter.colorMatrix(
                            colorMatrixTimes(saturationMatrix(saturation), hueMatrix(hue)),
                        )
                    },
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    BenchMarkers.tap()
                    blur = 0f; saturation = 1f; hue = 0f
                }, modifier = Modifier.testTag("filter:reset")) { Text("Reset") }
                Button(onClick = {
                    BenchMarkers.tap()
                    blur = 3f; saturation = 0.7f
                }, modifier = Modifier.testTag("filter:dreamy")) { Text("Dreamy") }
                Button(onClick = {
                    BenchMarkers.tap()
                    hue = 180f; saturation = 1.8f
                }, modifier = Modifier.testTag("filter:vibrant")) { Text("Vibrant") }
                Button(onClick = {
                    BenchMarkers.tap()
                    blur = 1f; saturation = 0.5f; hue = 30f
                }, modifier = Modifier.testTag("filter:vintage")) { Text("Vintage") }
            }
        }
    }
}
