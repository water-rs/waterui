package dev.waterui.android.reference

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.layout.ContentScale
import androidx.compose.foundation.Image
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import android.graphics.BitmapFactory

/**
 * Compose twin of the media_picker fixture: Pick Image / Pick Video /
 * Pick Live Photo buttons driving the Android photo picker, with the same
 * empty/error/selected display states the fixture shows.
 */
@Composable
fun MediaPickerTwin() {
    var selected by remember { mutableStateOf<Pair<Uri, String>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    @Composable
    fun launcher(kind: String, filter: androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia.VisualMediaType) =
        rememberLauncherForActivityResult(
            ActivityResultContracts.PickVisualMedia(),
        ) { uri ->
            BenchMarkers.tap()
            if (uri != null) {
                selected = uri to kind
                error = null
            }
        }

    val pickImage = launcher("Image", ActivityResultContracts.PickVisualMedia.ImageOnly)
    val pickVideo = launcher("Video", ActivityResultContracts.PickVisualMedia.VideoOnly)
    // Android's photo picker has no live-photo filter; the fixture's third
    // picker maps onto the image picker here, which is what a Compose app
    // would do.
    val pickLive = launcher("Live Photo", ActivityResultContracts.PickVisualMedia.ImageOnly)

    Column(
        Modifier.fillMaxSize().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(WATERUI_SPACING.dp),
    ) {
        TitleText("Media Picker Demo")

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = {
                    BenchMarkers.tap()
                    pickImage.launch(
                        PickVisualMediaRequest(
                            ActivityResultContracts.PickVisualMedia.ImageOnly,
                        ),
                    )
                },
                modifier = Modifier.testTag("picker:image"),
            ) { Text("Pick Image") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    pickVideo.launch(
                        PickVisualMediaRequest(
                            ActivityResultContracts.PickVisualMedia.VideoOnly,
                        ),
                    )
                },
                modifier = Modifier.testTag("picker:video"),
            ) { Text("Pick Video") }
            Button(
                onClick = {
                    BenchMarkers.tap()
                    pickLive.launch(
                        PickVisualMediaRequest(
                            ActivityResultContracts.PickVisualMedia.ImageOnly,
                        ),
                    )
                },
                modifier = Modifier.testTag("picker:live"),
            ) { Text("Pick Live Photo") }
        }

        Box(
            Modifier.fillMaxWidth().weight(1f)
                .clip(RoundedCornerShape(12.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                .testTag("picker:display"),
            contentAlignment = Alignment.Center,
        ) {
            val sel = selected
            val err = error
            when {
                err != null -> Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    SubheadlineText("Error")
                    BodyText(err)
                }
                sel != null -> Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                    modifier = Modifier.padding(8.dp),
                ) {
                    val context = LocalContext.current
                    val bitmap = remember(sel.first) {
                        runCatching {
                            context.contentResolver.openInputStream(sel.first)
                                ?.use { BitmapFactory.decodeStream(it) }
                        }.getOrNull()
                    }
                    if (bitmap != null) {
                        Image(
                            bitmap = bitmap.asImageBitmap(),
                            contentDescription = sel.second,
                            modifier = Modifier.fillMaxWidth().height(320.dp),
                            contentScale = ContentScale.Fit,
                        )
                    } else {
                        BodyText("${sel.second}: ${sel.first}")
                    }
                    FootnoteText(sel.second)
                }
                else -> Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    SubheadlineText("No media selected")
                    BodyText("Tap a button above to select media")
                }
            }
        }
    }
}
