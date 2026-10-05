// Competitive benchmark contestant — Jetpack Compose (water-rs/waterui#1262).
// Implements the canonical workload spec (benchmarks/competitive/README.md),
// the same constants the shared apps/* contestants render.

package dev.bench.compose

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// Canonical palette — identical in every contestant.
val palette = listOf(
    0xFF3B82F6, 0xFF10B981, 0xFFF59E0B, 0xFFEF4444, 0xFF8B5CF6, 0xFFEC4899,
).map(::Color)

fun timestamp(i: Int): String = "%02d:%02d".format((i / 60) % 24, i % 60)

// Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt, embedded
// (an app cannot read the suite's file at runtime).
val paragraphs = listOf(
    "The quick brown fox jumps over the lazy dog. 。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 。🌊 Fine-grained reactivity updates only the widgets that read the value.",
    "Almost all programming can be viewed as state management. ，。📚 Signals flow through the graph and wake the views that observe them.",
    "Sphinx of black quartz, judge my vow. のテキストもぜます。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! ，。🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. ，。🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. ，。🌲 Lazily built lists keep memory flat while content grows without bound.",
    "The five boxing wizards jump quickly. ，。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. ，。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
)

// Canonical W3/W5 geometry: rects wander a fixed 720x440 logical field.
const val FIELD_W = 720
const val FIELD_H = 440
const val RECT = 40

class XorShift64(private var s: Long) {
    fun next01(): Float {
        s = s xor (s shl 13); s = s xor (s ushr 7); s = s xor (s shl 17)
        return (s % 10_000).toFloat() / 10_000f
    }
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Missing or unrecognized workload traps — never silently render W1.
        val workload = intent.getStringExtra("workload")?.uppercase()
            ?: throw IllegalStateException(
                "missing 'workload' intent extra; expected W1..W6")
        val step = intent.getIntExtra("step", 0)
        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    when (workload) {
                        "W1" -> W1Hello()
                        "W2" -> W2Feed()
                        "W3" -> W3Motion(200)
                        "W4" -> W4Text()
                        "W5" -> W3Motion(step)
                        "W6" -> W6FeedCapacity(step)
                        else -> throw IllegalStateException(
                            "unrecognized workload extra '$workload'")
                    }
                }
            }
        }
    }
}

// W1 Hello — one centred label and one button that increments a counter.
@Composable
fun W1Hello() {
    var count by remember { mutableIntStateOf(0) }
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text("Count: $count", fontSize = 20.sp)
            Button(onClick = { count++ }) { Text("Increment") }
        }
    }
}

// W2 Feed — lazily built list of 10,000 rows (canonical row content).
@Composable
fun FeedRow(i: Int, complexity: Int = 0) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            modifier = Modifier
                .size(40.dp)
                .clip(CircleShape)
                .background(palette[i % palette.size])
        )
        Column(modifier = Modifier.weight(1f)) {
            Text("Row title $i", fontSize = 16.sp)
            Text(
                "Second line of subtitle for item $i",
                fontSize = 13.sp,
                maxLines = 1,
            )
        }
        // W6's per-row load: `complexity` sibling cells of a small rounded
        // rect plus a "c{j}" caption, between the text column and the
        // timestamp — same placement as every other contestant.
        for (j in 0 until complexity) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Box(
                    modifier = Modifier
                        .size(14.dp)
                        .clip(RoundedCornerShape(4.dp))
                        .background(palette[(i + j) % palette.size])
                )
                Text("c$j", fontSize = 12.sp)
            }
        }
        Text(timestamp(i), fontSize = 13.sp)
    }
}

@Composable
fun W2Feed() {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        items(10_000) { i -> FeedRow(i) }
    }
}

// W3 Motion / W5 capacity — `count` rects wander the field, each on its
// own xorshift64 sequence and period (1200 + (i%5)*200 ms, ease-in-out).
@Composable
fun W3Motion(count: Int) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Box(modifier = Modifier.size(FIELD_W.dp, FIELD_H.dp)) {
            for (i in 0 until count) {
                WanderRect(i)
            }
        }
    }
}

@Composable
fun WanderRect(i: Int) {
    val durMs = 1200 + (i % 5) * 200
    val x = remember { Animatable(0f) }
    val y = remember { Animatable(0f) }
    val rot = remember { Animatable(0f) }
    val opa = remember { Animatable(1f) }
    LaunchedEffect(i) {
        // Initial state seeded from the init stream; drive stream owns ticks.
        val init = XorShift64(
            0xD1B54A32D192ED03uL.toLong() xor i.toLong() * 0x2545F4914F6CDD1DuL.toLong())
        x.snapTo(init.next01() * (FIELD_W - RECT))
        y.snapTo(init.next01() * (FIELD_H - RECT))
        rot.snapTo(init.next01() * 360f)
        opa.snapTo(0.3f + init.next01() * 0.7f)
        val rng = XorShift64(
            0x9E3779B97F4A7C15uL.toLong() xor i.toLong() * 0xBF58476D1CE4E5B9uL.toLong())
        while (true) {
            val tx = rng.next01() * (FIELD_W - RECT)
            val ty = rng.next01() * (FIELD_H - RECT)
            val tr = rng.next01() * 360f
            val to = 0.3f + rng.next01() * 0.7f
            // All four channels move on the same tick like the other
            // contestants; coroutines let each Animatable run in parallel.
            kotlinx.coroutines.coroutineScope {
                kotlinx.coroutines.launch {
                    x.animateTo(tx, tween(durMs, easing = EaseInOut))
                }
                kotlinx.coroutines.launch {
                    y.animateTo(ty, tween(durMs, easing = EaseInOut))
                }
                kotlinx.coroutines.launch {
                    rot.animateTo(tr, tween(durMs, easing = EaseInOut))
                }
                kotlinx.coroutines.launch {
                    opa.animateTo(to, tween(durMs, easing = EaseInOut))
                }
            }
        }
    }
    Box(
        modifier = Modifier
            .offset(x.value.dp, y.value.dp)
            .rotate(rot.value)
            .alpha(opa.value)
            .size(RECT.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(palette[i % palette.size])
    )
}

// W6 Feed capacity — the W2 feed, each row carrying `complexity` sibling
// cells (canonical model: extras appended after the text column).
@Composable
fun W6FeedCapacity(complexity: Int) {
    val c = if (complexity <= 0) 1 else complexity
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        items(10_000) { i -> FeedRow(i, c) }
    }
}

// W4 Text — a scrolling screen of 50 paragraphs of mixed Latin/CJK/emoji.
@Composable
fun W4Text() {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        for (i in 0 until 50) {
            Text(paragraphs[i % paragraphs.size], fontSize = 16.sp)
        }
    }
}
