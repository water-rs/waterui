package dev.waterui.hydrolysis.hwui

import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RenderNode
import android.graphics.Typeface
import android.graphics.fonts.Font
import android.graphics.fonts.FontFamily
import android.graphics.fonts.FontStyle
import android.icu.text.BreakIterator
import android.text.Layout
import android.text.SpannableString
import android.text.Spanned
import android.text.StaticLayout
import android.text.TextDirectionHeuristics
import android.text.TextPaint
import android.text.TextUtils
import android.text.style.BackgroundColorSpan
import android.text.style.LineHeightSpan
import android.text.style.MetricAffectingSpan
import java.nio.ByteBuffer
import java.util.Locale

/** The platform text layouts a frame's `Text` op draws, by layout id. */
interface TextLayouts {
    /** Layout `id`'s node, recorded once with `Layout.draw`. */
    fun node(id: Int): RenderNode

    /** Drops layout `id`, on the frame's `ReleaseTextLayout`. */
    fun release(id: Int)
}

/**
 * The Android text engine's platform half. The Rust `HwuiTextLayout`
 * shapes each layout with one [shape] call that builds a `StaticLayout`
 * from the resolved runs, draws it once with `Layout.draw` into its own
 * `RenderNode` and answers its metrics and ink in one packed reply; caret,
 * hit-test, selection and navigation queries run against the live layout.
 *
 * Offsets are UTF-16. Shaping and queries arrive on the text engine's
 * threads and releases on the UI thread, so every entry point holds the
 * provider's lock.
 */
class HwuiTextProvider : TextLayouts {
    private val lock = Any()
    private val families = HashMap<String, MutableList<Font>>()
    private val typefaces = HashMap<String, Typeface>()
    private val layouts = Table<Layout>("text layout")
    private val nodes = Table<RenderNode>("text layout node")
    private val inkPaint = TextPaint(TextPaint.ANTI_ALIAS_FLAG)
    private val bounds = Rect()

    /**
     * Registers an app font given as bytes under `family`. Families
     * resolve through `Typeface.CustomFallbackBuilder` with the system
     * fallback.
     */
    fun registerFont(family: String, bytes: ByteBuffer, weight: Int, italic: Boolean, ttcIndex: Int) {
        // `bytes` views Rust memory for this call only; the font keeps a copy.
        val copy = ByteBuffer.allocateDirect(bytes.remaining())
        copy.put(bytes.duplicate())
        copy.flip()
        val slant = if (italic) FontStyle.FONT_SLANT_ITALIC else FontStyle.FONT_SLANT_UPRIGHT
        val font = Font.Builder(copy).setWeight(weight).setSlant(slant).setTtcIndex(ttcIndex).build()
        synchronized(lock) {
            families.getOrPut(family) { ArrayList() }.add(font)
            typefaces.remove(family)
        }
    }

    /**
     * Shapes layout `id`; see `hwui::text::wire` for the request and reply.
     * `familyLists` holds each run's family list, names joined by
     * [TextWire.FAMILY_SEPARATOR]; `paragraph` carries the alignment, the
     * direction, the ellipsis and strict family resolution.
     */
    fun shape(
        id: Int,
        text: String,
        spanCount: Int,
        spans: IntArray,
        familyLists: Array<String>,
        locale: String,
        maxWidth: Float,
        maxLines: Int,
        paragraph: Int,
    ): FloatArray =
        synchronized(lock) {
            require(spans.size == spanCount * TextWire.SPAN_WORDS) {
                "$spanCount style runs packed into ${spans.size} ints"
            }
            require(maxLines == TextWire.NO_LINE_LIMIT || maxLines > 0) { "a limit of $maxLines lines" }
            check(!layouts.contains(id)) { "text layout $id is already live" }
            val strict = paragraph and TextWire.STRICT_FAMILIES != 0
            val styled = SpannableString(text)
            for (run in 0 until spanCount) {
                style(styled, spans, run * TextWire.SPAN_WORDS, familyLists, strict)
            }
            val paint = TextPaint(TextPaint.ANTI_ALIAS_FLAG)
            paint.textLocale = if (locale.isEmpty()) Locale.ROOT else Locale.forLanguageTag(locale)
            val width =
                if (maxWidth == TextWire.UNBOUNDED) {
                    kotlin.math.ceil(Layout.getDesiredWidth(styled, paint)).toInt()
                } else {
                    kotlin.math.ceil(maxWidth).toInt()
                }
            val rightToLeft = paragraph and TextWire.RIGHT_TO_LEFT != 0
            val builder =
                StaticLayout.Builder.obtain(styled, 0, styled.length, paint, width)
                    .setAlignment(alignment(paragraph and TextWire.ALIGN_MASK))
                    .setTextDirection(if (rightToLeft) TextDirectionHeuristics.RTL else TextDirectionHeuristics.LTR)
                    .setIncludePad(false)
                    .setUseLineSpacingFromFallbacks(true)
            if (maxLines != TextWire.NO_LINE_LIMIT) builder.setMaxLines(maxLines)
            if (paragraph and TextWire.ELLIPSIS != 0) {
                require(maxLines != TextWire.NO_LINE_LIMIT) { "an ellipsis without a line limit" }
                builder.setEllipsize(TextUtils.TruncateAt.END).setEllipsizedWidth(width)
            }
            val layout = builder.build()
            val node = RenderNode("hydrolysis-text#$id")
            node.setPosition(0, 0, layout.width, layout.height)
            // Glyph ink may overhang the line boxes; the reply's bounds cover it.
            node.setClipToBounds(false)
            val canvas = node.beginRecording(layout.width, layout.height)
            try {
                layout.draw(canvas)
            } finally {
                node.endRecording()
            }
            val reply = reply(layout, styled)
            layouts.put(id, layout)
            nodes.put(id, node)
            reply
        }

    override fun node(id: Int): RenderNode = synchronized(lock) { nodes[id] }

    override fun release(id: Int) {
        synchronized(lock) {
            // Dropping the references frees the layout and its display list.
            layouts.remove(id)
            nodes.remove(id)
        }
    }

    /** The caret at `offset` as left, top, right, bottom. */
    fun caretRect(id: Int, offset: Int, upstream: Boolean): FloatArray =
        synchronized(lock) {
            val layout = layouts[id]
            val downstream = layout.getLineForOffset(offset)
            val line = if (upstream && downstream > 0 && offset == layout.getLineStart(downstream)) downstream - 1 else downstream
            val x =
                when {
                    line == downstream -> layout.getPrimaryHorizontal(offset)
                    layout.getParagraphDirection(line) == Layout.DIR_RIGHT_TO_LEFT -> layout.getLineLeft(line)
                    else -> layout.getLineRight(line)
                }
            floatArrayOf(x, layout.getLineTop(line).toFloat(), x + CARET_WIDTH, layout.getLineBottom(line).toFloat())
        }

    /** The position a point hits, packed with [TextWire.packPosition]. */
    fun hitTest(id: Int, x: Float, y: Float): Long =
        synchronized(lock) {
            val layout = layouts[id]
            val line = layout.getLineForVertical(y.toInt())
            val offset = layout.getOffsetForHorizontal(line, x)
            val upstream = offset == layout.getLineEnd(line) && (line < layout.lineCount - 1 || offset == layout.text.length)
            TextWire.packPosition(offset, upstream)
        }

    /** The word under a point, packed with [TextWire.packRange]. */
    fun wordAt(id: Int, x: Float, y: Float): Long =
        synchronized(lock) {
            val layout = layouts[id]
            val line = layout.getLineForVertical(y.toInt())
            val offset = layout.getOffsetForHorizontal(line, x)
            val words = BreakIterator.getWordInstance()
            words.setText(layout.text)
            val length = layout.text.length
            val start = if (offset >= length) words.preceding(length) else if (words.isBoundary(offset)) offset else words.preceding(offset)
            val end = if (offset >= length) length else words.following(offset)
            TextWire.packRange(maxOf(start, 0), if (end == BreakIterator.DONE) length else end)
        }

    /** The line under a point, packed with [TextWire.packRange]. */
    fun lineAt(id: Int, x: Float, y: Float): Long =
        synchronized(lock) {
            val layout = layouts[id]
            val line = layout.getLineForVertical(y.toInt())
            TextWire.packRange(layout.getLineStart(line), layout.getLineEnd(line))
        }

    /** The cluster boundary at or before `offset`. */
    fun snap(id: Int, offset: Int): Int =
        synchronized(lock) {
            val layout = layouts[id]
            val length = layout.text.length
            if (offset <= 0 || offset >= length) {
                offset.coerceIn(0, length)
            } else {
                layout.paint.getTextRunCursor(
                    layout.text, 0, length, layout.isRtlCharAt(offset), offset, Paint.CURSOR_AT_OR_BEFORE,
                )
            }
        }

    /** The rectangles covering `start` until `end`, four floats each, one per line. */
    fun selectionRects(id: Int, start: Int, end: Int): FloatArray =
        synchronized(lock) {
            val layout = layouts[id]
            val first = layout.getLineForOffset(start)
            val last = layout.getLineForOffset(end)
            val out = FloatArray(4 * (last - first + 1))
            for (line in first..last) {
                val from = maxOf(start, layout.getLineStart(line))
                val to = minOf(end, layout.getLineEnd(line))
                val a = layout.getPrimaryHorizontal(from)
                val b = if (to == layout.getLineEnd(line) && line < layout.lineCount - 1) {
                    if (layout.getParagraphDirection(line) == Layout.DIR_RIGHT_TO_LEFT) layout.getLineLeft(line) else layout.getLineRight(line)
                } else {
                    layout.getPrimaryHorizontal(to)
                }
                val at = 4 * (line - first)
                out[at] = minOf(a, b)
                out[at + 1] = layout.getLineTop(line).toFloat()
                out[at + 2] = maxOf(a, b)
                out[at + 3] = layout.getLineBottom(line).toFloat()
            }
            out
        }

    /** The offset one cluster to the left of `offset`. */
    fun previousVisual(id: Int, offset: Int): Int = synchronized(lock) { layouts[id].getOffsetToLeftOf(offset) }

    /** The offset one cluster to the right of `offset`. */
    fun nextVisual(id: Int, offset: Int): Int = synchronized(lock) { layouts[id].getOffsetToRightOf(offset) }

    private fun style(styled: SpannableString, spans: IntArray, at: Int, familyLists: Array<String>, strict: Boolean) {
        val start = spans[at + TextWire.START]
        val end = spans[at + TextWire.END]
        require(start in 0..end && end <= styled.length) { "style run $start..$end in a text of ${styled.length} units" }
        val flags = spans[at + TextWire.FLAGS]
        val family = spans[at + TextWire.FAMILY]
        val base = if (family == TextWire.DEFAULT_FAMILY) Typeface.DEFAULT else typeface(familyLists[family], strict)
        val typeface = Typeface.create(base, spans[at + TextWire.WEIGHT], flags and TextWire.ITALIC != 0)
        val foreground = if (flags and TextWire.HAS_FOREGROUND != 0) TextWire.colorLong(spans, at + TextWire.FOREGROUND) else null
        val run =
            RunSpan(
                typeface,
                Float.fromBits(spans[at + TextWire.SIZE]),
                Float.fromBits(spans[at + TextWire.LETTER_SPACING]),
                flags and TextWire.UNDERLINE != 0,
                flags and TextWire.STRIKETHROUGH != 0,
                foreground,
            )
        styled.setSpan(run, start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        if (flags and TextWire.HAS_BACKGROUND != 0) {
            // `Layout` paints span backgrounds from an sRGB int colour.
            val background = Color.toArgb(TextWire.colorLong(spans, at + TextWire.BACKGROUND))
            styled.setSpan(BackgroundColorSpan(background), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
        if (flags and TextWire.HAS_LINE_HEIGHT != 0) {
            val height = kotlin.math.round(Float.fromBits(spans[at + TextWire.LINE_HEIGHT])).toInt()
            styled.setSpan(LineHeightSpan.Standard(height), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
    }

    /**
     * The typeface of a family list, in order: registered app families
     * chain through `Typeface.CustomFallbackBuilder`; the first system
     * family ends the chain as its system fallback (`sans-serif` when the
     * list names none). A name that is neither is an error when `strict`,
     * and skipped otherwise, as a CSS family list skips it.
     */
    private fun typeface(list: String, strict: Boolean): Typeface =
        typefaces.getOrPut(if (strict) "!$list" else list) {
            var chain: Typeface.CustomFallbackBuilder? = null
            var system: Typeface? = null
            var systemName = DEFAULT_SYSTEM_FAMILY
            for (name in list.split(TextWire.FAMILY_SEPARATOR)) {
                val fonts = families[name]
                if (fonts != null) {
                    val family = FontFamily.Builder(fonts[0])
                    for (font in fonts.subList(1, fonts.size)) family.addFont(font)
                    chain = chain?.addCustomFallback(family.build()) ?: Typeface.CustomFallbackBuilder(family.build())
                    continue
                }
                val resolved = SystemFamilies.resolve(name)
                if (resolved != null) {
                    system = resolved
                    systemName = name
                    break
                }
                require(!strict) { "font family `$name` is neither registered nor a system family" }
            }
            chain?.setSystemFallback(systemName)?.build() ?: system ?: Typeface.DEFAULT
        }

    private fun alignment(code: Int): Layout.Alignment =
        when (code) {
            TextWire.ALIGN_START -> Layout.Alignment.ALIGN_NORMAL
            TextWire.ALIGN_CENTER -> Layout.Alignment.ALIGN_CENTER
            TextWire.ALIGN_END -> Layout.Alignment.ALIGN_OPPOSITE
            else -> throw IllegalArgumentException("text alignment code $code")
        }

    private fun reply(layout: Layout, styled: Spanned): FloatArray {
        val lines = layout.lineCount
        val out = FloatArray(TextWire.REPLY_HEADER + TextWire.LINE_WORDS * lines)
        out[0] = layout.width.toFloat()
        out[1] = layout.height.toFloat()
        out[2] = Float.POSITIVE_INFINITY
        out[3] = Float.NEGATIVE_INFINITY
        for (line in 0 until lines) {
            val at = TextWire.REPLY_HEADER + TextWire.LINE_WORDS * line
            out[at] = layout.getLineWidth(line)
            out[at + 1] = (layout.getLineBottom(line) - layout.getLineTop(line)).toFloat()
            out[at + 2] = layout.getLineBaseline(line).toFloat()
            lineInk(layout, styled, line, out, at + 3)
        }
        return out
    }

    /**
     * The horizontal glyph bounds of `line` into `out[at]`, `out[at + 1]`,
     * widening the layout's vertical ink in `out[2]`, `out[3]`; measured per
     * segment of one style and one direction with that segment's paint.
     */
    private fun lineInk(layout: Layout, styled: Spanned, line: Int, out: FloatArray, at: Int) {
        var left = Float.POSITIVE_INFINITY
        var right = Float.NEGATIVE_INFINITY
        val end = layout.getLineVisibleEnd(line)
        var from = layout.getLineStart(line)
        while (from < end) {
            val styleEnd = styled.nextSpanTransition(from, end, MetricAffectingSpan::class.java)
            val rtl = layout.isRtlCharAt(from)
            var to = from + 1
            while (to < styleEnd && layout.isRtlCharAt(to) == rtl) to++
            inkPaint.set(layout.paint)
            for (span in styled.getSpans(from, to, MetricAffectingSpan::class.java)) span.updateMeasureState(inkPaint)
            inkPaint.getTextBounds(styled, from, to, bounds)
            if (!bounds.isEmpty) {
                val pen = layout.getPrimaryHorizontal(from)
                val origin = if (rtl) pen - inkPaint.measureText(styled, from, to) else pen
                left = minOf(left, origin + bounds.left)
                right = maxOf(right, origin + bounds.right)
                val baseline = layout.getLineBaseline(line)
                out[2] = minOf(out[2], (baseline + bounds.top).toFloat())
                out[3] = maxOf(out[3], (baseline + bounds.bottom).toFloat())
            }
            from = to
        }
        out[at] = left
        out[at + 1] = right
    }

    /** One style run: the measured attributes, and the drawn ones on top. */
    private class RunSpan(
        private val typeface: Typeface,
        private val size: Float,
        private val letterSpacing: Float,
        private val underline: Boolean,
        private val strikethrough: Boolean,
        private val foreground: Long?,
    ) : MetricAffectingSpan() {
        override fun updateMeasureState(paint: TextPaint) {
            paint.typeface = typeface
            paint.textSize = size
            paint.letterSpacing = letterSpacing
        }

        override fun updateDrawState(paint: TextPaint) {
            updateMeasureState(paint)
            paint.isUnderlineText = underline
            paint.isStrikeThruText = strikethrough
            if (foreground != null) paint.setColor(foreground)
        }
    }

    private companion object {
        const val CARET_WIDTH = 1f
        const val DEFAULT_SYSTEM_FAMILY = "sans-serif"
    }
}
