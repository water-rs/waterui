// Competitive benchmark contestant — Android Views (water-rs/waterui#1262).
// Implements the canonical workload spec (benchmarks/competitive/README.md),
// the same constants the shared apps/* contestants render.

package dev.bench.views

import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.animation.ValueAnimator
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.animation.AccelerateDecelerateInterpolator
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.app.Activity
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import kotlin.math.roundToInt

// Canonical palette — identical in every contestant.
val palette = intArrayOf(
    0xFF3B82F6.toInt(), 0xFF10B981.toInt(), 0xFFF59E0B.toInt(),
    0xFFEF4444.toInt(), 0xFF8B5CF6.toInt(), 0xFFEC4899.toInt(),
)

fun timestamp(i: Int): String = "%02d:%02d".format((i / 60) % 24, i % 60)

// Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt, embedded
// (an app cannot read the suite's file at runtime).
val paragraphs = arrayOf(
    "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳過懶惰的狗。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 水のインターフェースはネイティブウィジェットを描画する。🌊",
    "Almost all programming can be viewed as state management. 几乎所有的编程都可以视为状态管理。📚 Signals flow through the graph.",
    "Sphinx of black quartz, judge my vow. 黒い水晶のスフィンクス、私の誓いを裁け。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! 빠른 얼룩말이 얼마나 성가시게 뛰는가! 🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. 밝은 여우가 뛰고 졸린 새가 꽥꽥 운다. 🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "ベンチマークが正直であれば最適化も正直になる。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. 두 명의 조키가 내 큰 퀴즈를 팩스로 보내는 것을 돕는다. 🌲 Lazily built lists keep memory flat.",
    "The five boxing wizards jump quickly. 五個拳擊巫師跳得很快。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. 寒鸦喜欢我巨大的石英斯芬克斯。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
)

// Canonical W3/W5 geometry: rects wander a fixed 720x440 logical field.
const val FIELD_W = 720f
const val FIELD_H = 440f
const val RECT = 40f

// xorshift64 on unsigned-64-bit state (WORKLOADS.md): `ushr` is the
// logical shift; the Kotlin Long is two's-complement u64 bits, so
// `u64 % 10000` is computed in two 32-bit halves (2^32 mod 10000 = 7296) —
// a signed `%` on a high-bit state would diverge from the shared sequence.
class XorShift64(private var s: Long) {
    fun next01(): Float {
        s = s xor (s shl 13); s = s xor (s ushr 7); s = s xor (s shl 17)
        val hi = s ushr 32
        val lo = s and 0xFFFFFFFFL
        val u = ((hi % 10_000) * 7296 + (lo % 10_000)) % 10_000
        return u.toFloat() / 10_000f
    }
}

// Capacity ladders — the runner's per-launch step must name a member;
// anything else is a failed launch, not a silent default.
val W5_STEPS = listOf(200, 400, 800, 1600, 3200, 6400, 12800, 25600)
val W6_STEPS = listOf(1, 2, 4, 8, 16, 32, 64)

fun ladderStep(workload: String, step: Int?, ladder: List<Int>): Int {
    if (step == null || step !in ladder) {
        throw IllegalStateException(
            "$workload: missing or unrecognized 'step' intent extra " +
            "(got $step); expected one of $ladder")
    }
    return step
}

class MainActivity : Activity() {
    private fun Int.dp(): Int = (this * resources.displayMetrics.density).roundToInt()
    private fun Float.dp(): Int = (this * resources.displayMetrics.density).roundToInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Workload ids are exact lowercase strings — anything else traps.
        val workload = intent.getStringExtra("workload")
            ?: throw IllegalStateException(
                "missing 'workload' intent extra; expected w1..w6")
        if (workload !in setOf("w1", "w2", "w3", "w4", "w5", "w6"))
            throw IllegalStateException(
                "unrecognized workload extra '$workload'; expected w1..w6")
        // W5/W6 pin one ladder step per launch — a missing or
        // out-of-ladder step traps, never silently a wrong count.
        val step = if (intent.hasExtra("step")) intent.getIntExtra("step", -1) else null
        when (workload) {
            "w1" -> w1()
            "w2" -> w2()
            "w3" -> w3(200)
            "w4" -> w4()
            "w5" -> w3(ladderStep("w5", step, W5_STEPS))
            "w6" -> w6(ladderStep("w6", step, W6_STEPS))
            else -> throw IllegalStateException(
                "unreachable: workload validated as w1..w6")
        }
    }

    // W1 Hello — one centred label and one button that increments a counter.
    private fun w1() {
        var count = 0
        val label = TextView(this).apply { textSize = 20f }
        fun render() { label.text = "Count: $count" }
        render()
        val button = Button(this).apply {
            text = "Increment"
            setOnClickListener { count++; render() }
        }
        val col = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            addView(label)
            addView(button, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = 16.dp() })
        }
        setContentView(col)
    }

    private fun newRow(): LinearLayout {
        val avatar = View(this).apply {
            layoutParams = LinearLayout.LayoutParams(40.dp(), 40.dp())
        }
        val texts = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            layoutParams = LinearLayout.LayoutParams(
                0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f,
            ).apply { marginStart = 12.dp() }
            addView(TextView(this@MainActivity).apply {
                tag = "title"; textSize = 16f
            })
            addView(TextView(this@MainActivity).apply {
                tag = "subtitle"; textSize = 13f
                maxLines = 1
                ellipsize = android.text.TextUtils.TruncateAt.END
            }, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = 4.dp() })
        }
        val extras = LinearLayout(this).apply {
            tag = "extras"; orientation = LinearLayout.HORIZONTAL
        }
        val ts = TextView(this).apply { tag = "ts"; textSize = 13f }
        return LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(16.dp(), 10.dp(), 16.dp(), 10.dp())
            addView(avatar)
            addView(texts)
            addView(extras, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { marginStart = 8.dp(); marginEnd = 12.dp() })
            addView(ts)
        }
    }

    private fun bindRow(row: LinearLayout, i: Int, complexity: Int) {
        (row.getChildAt(0) as View).background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(palette[i % palette.size])
        }
        val texts = row.getChildAt(1) as LinearLayout
        texts.findViewWithTag<TextView>("title").text = "Row title $i"
        texts.findViewWithTag<TextView>("subtitle").text =
            "Second line of subtitle for item $i"
        // W6 cells are reused on bind — the count is fixed for the
        // launch, so rebinds repaint each cell rather than rebuild it.
        val extras = row.findViewWithTag<LinearLayout>("extras")
        while (extras.childCount < complexity) {
            val cell = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.CENTER_HORIZONTAL
                tag = "cell"
                addView(View(this@MainActivity), LinearLayout.LayoutParams(
                    14.dp(), 14.dp()))
                addView(TextView(this@MainActivity).apply { textSize = 12f })
            }
            extras.addView(cell, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { marginStart = 4.dp() })
        }
        while (extras.childCount > complexity) {
            extras.removeViewAt(extras.childCount - 1)
        }
        for (j in 0 until complexity) {
            val cell = extras.getChildAt(j) as LinearLayout
            cell.getChildAt(0).background = GradientDrawable().apply {
                cornerRadius = 4 * resources.displayMetrics.density
                setColor(palette[(i + j) % palette.size])
            }
            (cell.getChildAt(1) as TextView).text = "c$j"
        }
        row.findViewWithTag<TextView>("ts").text = timestamp(i)
    }

    // W2 Feed — lazily built list of 10,000 rows via RecyclerView.
    private fun w2() {
        val rv = RecyclerView(this).apply {
            layoutManager = LinearLayoutManager(this@MainActivity)
            adapter = FeedAdapter()
        }
        setContentView(rv)
    }

    inner class FeedAdapter : RecyclerView.Adapter<FeedAdapter.Holder>() {
        inner class Holder(val row: LinearLayout) : RecyclerView.ViewHolder(row)

        override fun getItemCount(): Int = 10_000
        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int) =
            Holder(newRow())
        override fun onBindViewHolder(holder: Holder, i: Int) =
            bindRow(holder.row, i, 0)
    }

    // W3 Motion / W5 capacity — `n` rects wander the field on per-rect
    // xorshift64 waypoint sequences, each tick ease-in-out over
    // (1200 + (i%5)*200) ms.
    private fun w3(n: Int) {
        val stage = FrameLayout(this).apply { clipChildren = false }
        val root = FrameLayout(this)
        root.addView(stage, FrameLayout.LayoutParams(
            FIELD_W.dp(), FIELD_H.dp(), Gravity.CENTER,
        ))

        val d = resources.displayMetrics.density
        for (i in 0 until n) {
            val init = XorShift64(
                0xD1B54A32D192ED03uL.toLong() xor
                    i.toLong() * 0x2545F4914F6CDD1DuL.toLong())
            val rng = XorShift64(
                0x9E3779B97F4A7C15uL.toLong() xor
                    i.toLong() * 0xBF58476D1CE4E5B9uL.toLong())
            val durMs = (1200 + (i % 5) * 200).toLong()
            val v = View(this).apply {
                background = GradientDrawable().apply {
                    cornerRadius = 10 * d
                    setColor(palette[i % palette.size])
                }
                translationX = init.next01() * (FIELD_W - RECT) * d
                translationY = init.next01() * (FIELD_H - RECT) * d
                rotation = init.next01() * 360f
                alpha = 0.3f + init.next01() * 0.7f
            }
            stage.addView(v, FrameLayout.LayoutParams(RECT.dp(), RECT.dp()))

            // One animator retargets all four channels per tick — position,
            // rotation and opacity land on the same new waypoint together.
            fun tick() {
                val sx = v.translationX / d; val sy = v.translationY / d
                val sr = v.rotation; val so = v.alpha
                val tx = rng.next01() * (FIELD_W - RECT)
                val ty = rng.next01() * (FIELD_H - RECT)
                val tr = rng.next01() * 360f
                val to = 0.3f + rng.next01() * 0.7f
                ValueAnimator.ofFloat(0f, 1f).apply {
                    duration = durMs
                    interpolator = AccelerateDecelerateInterpolator()
                    addUpdateListener {
                        val f = it.animatedValue as Float
                        v.translationX = (sx + (tx - sx) * f) * d
                        v.translationY = (sy + (ty - sy) * f) * d
                        v.rotation = sr + (tr - sr) * f
                        v.alpha = so + (to - so) * f
                    }
                    addListener(object : AnimatorListenerAdapter() {
                        override fun onAnimationEnd(animator: Animator) {
                            if (!v.isAttachedToWindow) return
                            tick()
                        }
                    })
                    start()
                }
            }
            v.addOnAttachStateChangeListener(
                object : View.OnAttachStateChangeListener {
                    override fun onViewAttachedToWindow(view: View) { tick() }
                    override fun onViewDetachedFromWindow(view: View) {}
                })
            if (v.isAttachedToWindow) tick()
        }
        setContentView(root)
    }

    // W6 Feed capacity — the W2 feed, each row carrying `complexity`
    // sibling cells (canonical model: extras appended after the text column).
    private fun w6(complexity0: Int) {
        val complexity = complexity0
        val rv = RecyclerView(this).apply {
            layoutManager = LinearLayoutManager(this@MainActivity)
            adapter = DeepFeedAdapter(complexity)
        }
        setContentView(rv)
    }

    inner class DeepFeedAdapter(private val complexity: Int) :
        RecyclerView.Adapter<DeepFeedAdapter.Holder>() {
        inner class Holder(val row: LinearLayout) : RecyclerView.ViewHolder(row)

        override fun getItemCount(): Int = 10_000
        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int) =
            Holder(newRow())
        override fun onBindViewHolder(holder: Holder, i: Int) =
            bindRow(holder.row, i, complexity)
    }

    // W4 Text — a scrolling screen of 50 paragraphs of mixed Latin/CJK/emoji.
    private fun w4() {
        val col = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        for (i in 0 until 50) {
            col.addView(TextView(this).apply {
                textSize = 16f
                setPadding(16.dp(), 10.dp(), 16.dp(), 10.dp())
                text = paragraphs[i % paragraphs.size]
            }, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { bottomMargin = 6.dp() })
        }
        setContentView(ScrollView(this).apply { addView(col) })
    }
}
