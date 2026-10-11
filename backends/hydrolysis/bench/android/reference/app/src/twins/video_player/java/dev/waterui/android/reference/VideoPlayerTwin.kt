package dev.waterui.android.reference

import android.media.MediaPlayer
import android.view.SurfaceHolder
import android.view.SurfaceView
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
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView

private data class Sample(val title: String, val profile: String, val asset: String)

// The fixture's five remote samples pin to the bundled clip; each pill still
// performs the full seek-to-item path (new MediaPlayer open) the controller
// runs for a source switch.
private val SAMPLES = listOf(
    Sample("Big Buck Bunny 1MB", "SDR / BT.709", "bench_clip.mp4"),
    Sample("Big Buck Bunny 5MB", "SDR / BT.709", "bench_clip.mp4"),
    Sample("Sintel", "SDR / BT.709", "bench_clip.mp4"),
    Sample("Jellyfin HDR10 1080p 3M", "HDR10 / BT.2020 + PQ", "bench_clip.mp4"),
    Sample("Jellyfin HDR10 1080p 10M", "HDR10 / BT.2020 + PQ", "bench_clip.mp4"),
)

/**
 * Compose twin of the video_player fixture: a native SurfaceView+MediaPlayer
 * (the same native video View family the Hydrolysis registry hosts) playing
 * the pinned local clip, with the buffering overlay, status line and the
 * five source pills keeping the fixture's titles and profiles.
 */
@Composable
fun VideoPlayerTwin() {
    var selected by remember { mutableIntStateOf(0) }
    var status by remember { mutableStateOf("Idle") }
    var buffering by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val player = remember { MediaPlayer() }

    DisposableEffect(Unit) {
        onDispose { player.release() }
    }

    fun load(index: Int) {
        status = "Buffering..."
        buffering = true
        runCatching {
            val afd = context.assets.openFd(SAMPLES[index].asset)
            player.reset()
            player.setDataSource(afd.fileDescriptor, afd.startOffset, afd.length)
            afd.close()
            player.prepareAsync()
        }.onFailure {
            status = "Error: ${it.message}"
            buffering = false
        }
    }

    Column(
        Modifier.fillMaxSize().padding(WATERUI_PADDING.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        HeadlineText("WaterUI Video Player")
        BodyText("Now Playing: ${SAMPLES[selected].title}")
        FootnoteText("Source Profile: ${SAMPLES[selected].profile}")

        Box(
            Modifier.fillMaxWidth().height(360.dp)
                .testTag("video:player"),
        ) {
            AndroidView(
                factory = { ctx ->
                    SurfaceView(ctx).apply {
                        holder.addCallback(
                            object : SurfaceHolder.Callback {
                                override fun surfaceCreated(h: SurfaceHolder) {
                                    player.setDisplay(h)
                                    player.setOnPreparedListener { mp ->
                                        buffering = false
                                        status = "Playing"
                                        mp.isLooping = true
                                        mp.start()
                                    }
                                    player.setOnInfoListener { _, what, _ ->
                                        when (what) {
                                            MediaPlayer.MEDIA_INFO_BUFFERING_START ->
                                                buffering = true
                                            MediaPlayer.MEDIA_INFO_BUFFERING_END -> {
                                                buffering = false
                                                status = "Playing"
                                            }
                                            else -> {}
                                        }
                                        true
                                    }
                                    player.setOnErrorListener { _, _, _ ->
                                        status = "Error: playback failed"
                                        buffering = false
                                        true
                                    }
                                    player.setOnCompletionListener {
                                        status = "Ended"
                                    }
                                    load(selected)
                                }

                                override fun surfaceChanged(
                                    h: SurfaceHolder, f: Int, w: Int, ht: Int,
                                ) = Unit

                                override fun surfaceDestroyed(h: SurfaceHolder) {
                                    player.setDisplay(null)
                                }
                            },
                        )
                    }
                },
                update = {},
                modifier = Modifier.fillMaxSize().background(Color.Black),
            )
            if (buffering) {
                Column(
                    Modifier.fillMaxSize().background(Color(0xCC000000)),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    CircularProgressIndicator()
                    Text("Buffering...", color = Color.White)
                }
            }
        }

        FootnoteText("Status: $status")

        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            for (row in listOf(0..2, 3..4)) {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    for (i in row) {
                        SourcePill(
                            label = SAMPLES[i].title,
                            selected = i == selected,
                            onClick = {
                                BenchMarkers.tap()
                                selected = i
                                load(i)
                            },
                            tag = "video:source-$i",
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SourcePill(label: String, selected: Boolean, onClick: () -> Unit, tag: String) {
    Surface(
        onClick = onClick,
        modifier = Modifier.testTag(tag),
        shape = RoundedCornerShape(50),
        color = if (selected) {
            Color.White.copy(alpha = 0.35f)
        } else {
            Color.White.copy(alpha = 0.15f)
        },
        contentColor = MaterialTheme.colorScheme.onSurface,
    ) {
        Text(
            label,
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}
