package dev.waterui.widgets

import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import android.text.TextUtils
import android.text.format.DateFormat
import android.text.format.DateUtils
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Chronometer
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.Space
import android.widget.TextView
import java.util.Date
import kotlin.math.roundToInt

/**
 * Native Android view realization for `WaterUIWidgetSchema.Node`.
 *
 * Maps every widget schema node (text with localization and live relative
 * dates, images, stacks, shapes, colors, padding, spacers, frames,
 * backgrounds, accessibility labels, links, and intents) onto standard Android
 * views and attributes so host providers and Glance/RemoteViews bridges share
 * one schema implementation.
 */
object WaterUIWidgetRenderer {
    const val ACTION_WIDGET_INTENT = "dev.waterui.widgets.ACTION_WIDGET_INTENT"
    const val EXTRA_INTENT_KIND = "dev.waterui.widgets.EXTRA_INTENT_KIND"
    const val EXTRA_INTENT_PAYLOAD = "dev.waterui.widgets.EXTRA_INTENT_PAYLOAD"

    private const val DEFAULT_PADDING_DP = 16f
    private const val RELATIVE_DATE_REFRESH_MS = 1_000L

    @JvmStatic
    fun render(context: Context, node: WaterUIWidgetSchema.Node): View {
        return when (node) {
            is WaterUIWidgetSchema.Node.Text -> renderText(context, node)
            is WaterUIWidgetSchema.Node.Image -> renderImage(context, node)
            is WaterUIWidgetSchema.Node.Stack -> renderStack(context, node)
            is WaterUIWidgetSchema.Node.Shape -> renderShape(context, node)
            is WaterUIWidgetSchema.Node.Color -> View(context).apply {
                setBackgroundColor(resolveColor(context, node.color))
            }
            is WaterUIWidgetSchema.Node.Padding -> renderPadding(context, node)
            is WaterUIWidgetSchema.Node.Spacer -> renderSpacer(context, node)
            is WaterUIWidgetSchema.Node.Frame -> renderFrame(context, node)
            is WaterUIWidgetSchema.Node.Background -> renderBackground(context, node)
            is WaterUIWidgetSchema.Node.AccessibilityLabel -> renderAccessibilityLabel(context, node)
            is WaterUIWidgetSchema.Node.Link -> renderLink(context, node)
            is WaterUIWidgetSchema.Node.Intent -> renderIntent(context, node)
        }
    }

    @JvmStatic
    fun renderTimelineEntry(context: Context, entry: WaterUIWidgetSchema.TimelineEntry): View {
        return render(context, entry.content)
    }

    private fun renderText(context: Context, node: WaterUIWidgetSchema.Node.Text): View {
        val textView: TextView = when (val content = node.content) {
            is WaterUIWidgetSchema.TextContent.Localized -> TextView(context).apply {
                text = resolveLocalizedString(context, content.key)
            }
            is WaterUIWidgetSchema.TextContent.Verbatim -> TextView(context).apply {
                text = content.text
            }
            is WaterUIWidgetSchema.TextContent.Date -> renderDateText(context, content)
        }

        applyFont(textView, node.font, node.bold)
        textView.setTextColor(
            resolveColor(
                context,
                node.color ?: WaterUIWidgetSchema.WidgetColor.Semantic(
                    WaterUIWidgetSchema.SemanticColor.PRIMARY,
                ),
            ),
        )
        val lineLimit = node.lineLimit
        if (lineLimit != null) {
            textView.maxLines = lineLimit
            textView.ellipsize = TextUtils.TruncateAt.END
        }
        return textView
    }

    private fun renderDateText(
        context: Context,
        content: WaterUIWidgetSchema.TextContent.Date,
    ): TextView {
        val targetMillis = (content.epochSeconds * 1000.0).toLong()
        return when (content.style) {
            WaterUIWidgetSchema.DateStyle.TIMER -> Chronometer(context).apply {
                val deltaMillis = targetMillis - System.currentTimeMillis()
                base = SystemClock.elapsedRealtime() + deltaMillis
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                    isCountDown = deltaMillis > 0L
                }
                start()
            }
            WaterUIWidgetSchema.DateStyle.RELATIVE,
            WaterUIWidgetSchema.DateStyle.OFFSET -> TextView(context).apply {
                val updateText = object : Runnable {
                    override fun run() {
                        text = DateUtils.getRelativeTimeSpanString(
                            targetMillis,
                            System.currentTimeMillis(),
                            DateUtils.SECOND_IN_MILLIS,
                            DateUtils.FORMAT_ABBREV_RELATIVE,
                        )
                        if (isAttachedToWindow) {
                            postDelayed(this, RELATIVE_DATE_REFRESH_MS)
                        }
                    }
                }
                updateText.run()
                addOnAttachStateChangeListener(object : View.OnAttachStateChangeListener {
                    override fun onViewAttachedToWindow(v: View) {
                        removeCallbacks(updateText)
                        updateText.run()
                    }

                    override fun onViewDetachedFromWindow(v: View) {
                        removeCallbacks(updateText)
                    }
                })
            }
            WaterUIWidgetSchema.DateStyle.DATE -> TextView(context).apply {
                text = DateFormat.getMediumDateFormat(context).format(Date(targetMillis))
            }
            WaterUIWidgetSchema.DateStyle.TIME -> TextView(context).apply {
                text = DateFormat.getTimeFormat(context).format(Date(targetMillis))
            }
        }
    }

    private fun renderImage(context: Context, node: WaterUIWidgetSchema.Node.Image): View {
        val imageView = ImageView(context)
        when (val source = node.source) {
            is WaterUIWidgetSchema.ImageSource.Data -> {
                val bitmap = BitmapFactory.decodeByteArray(source.bytes, 0, source.bytes.size)
                    ?: throw IllegalArgumentException("widget image data does not decode")
                imageView.setImageBitmap(bitmap)
            }
            is WaterUIWidgetSchema.ImageSource.SfSymbol -> {
                val drawableName = source.name.replace('.', '_').lowercase()
                val resId = context.resources.getIdentifier(
                    drawableName,
                    "drawable",
                    context.packageName,
                )
                if (resId != 0) {
                    imageView.setImageResource(resId)
                }
            }
        }
        val tint = node.color
        if (tint != null) {
            imageView.imageTintList = ColorStateList.valueOf(resolveColor(context, tint))
        }
        return imageView
    }

    private fun renderStack(context: Context, node: WaterUIWidgetSchema.Node.Stack): View {
        if (node.axis == WaterUIWidgetSchema.Axis.DEPTH) {
            val frame = FrameLayout(context)
            val gravity = resolveGravity(node.alignment, Gravity.CENTER)
            for (child in node.children) {
                val view = render(context, child)
                val params = FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    gravity,
                )
                frame.addView(view, params)
            }
            return frame
        }

        val isVertical = node.axis == WaterUIWidgetSchema.Axis.VERTICAL
        val layout = LinearLayout(context).apply {
            orientation = if (isVertical) LinearLayout.VERTICAL else LinearLayout.HORIZONTAL
            gravity = if (isVertical) {
                resolveHorizontalGravity(node.alignment?.horizontal)
            } else {
                resolveVerticalGravity(node.alignment?.vertical)
            }
        }
        val spacingPx = node.spacing?.let { dpToPx(context, it) } ?: 0
        for ((index, child) in node.children.withIndex()) {
            val view = render(context, child)
            val params = if (child is WaterUIWidgetSchema.Node.Spacer) {
                if (isVertical) {
                    LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, 0, 1f)
                } else {
                    LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
                }
            } else {
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                )
            }
            if (index > 0 && spacingPx > 0) {
                if (isVertical) {
                    params.topMargin = spacingPx
                } else {
                    params.marginStart = spacingPx
                }
            }
            layout.addView(view, params)
        }
        return layout
    }

    private fun renderShape(context: Context, node: WaterUIWidgetSchema.Node.Shape): View {
        val drawable = GradientDrawable().apply {
            setColor(resolveColor(context, node.fill))
            when (val kind = node.kind) {
                WaterUIWidgetSchema.ShapeKind.Rectangle -> shape = GradientDrawable.RECTANGLE
                is WaterUIWidgetSchema.ShapeKind.RoundedRectangle -> {
                    shape = GradientDrawable.RECTANGLE
                    cornerRadius = dpToPxFloat(context, kind.cornerRadius)
                }
                WaterUIWidgetSchema.ShapeKind.Capsule -> {
                    shape = GradientDrawable.RECTANGLE
                    cornerRadius = dpToPxFloat(context, 9999f)
                }
                WaterUIWidgetSchema.ShapeKind.Circle,
                WaterUIWidgetSchema.ShapeKind.Ellipse -> shape = GradientDrawable.OVAL
            }
        }
        return View(context).apply { background = drawable }
    }

    private fun renderPadding(context: Context, node: WaterUIWidgetSchema.Node.Padding): View {
        val container = FrameLayout(context)
        val insets = node.insets
        if (insets != null) {
            container.setPaddingRelative(
                dpToPx(context, insets.leading),
                dpToPx(context, insets.top),
                dpToPx(context, insets.trailing),
                dpToPx(context, insets.bottom),
            )
        } else {
            val defaultPx = dpToPx(context, DEFAULT_PADDING_DP)
            container.setPaddingRelative(defaultPx, defaultPx, defaultPx, defaultPx)
        }
        container.addView(
            render(context, node.content),
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
        return container
    }

    private fun renderSpacer(context: Context, node: WaterUIWidgetSchema.Node.Spacer): View {
        return Space(context).apply {
            val minPx = node.minLength?.let { dpToPx(context, it) }
            if (minPx != null) {
                minimumWidth = minPx
                minimumHeight = minPx
            }
        }
    }

    private fun renderFrame(context: Context, node: WaterUIWidgetSchema.Node.Frame): View {
        val container = FrameLayout(context)
        val widthSpec = when {
            node.width != null -> dpToPx(context, node.width)
            node.maxWidth is WaterUIWidgetSchema.MaxLength.Infinity -> ViewGroup.LayoutParams.MATCH_PARENT
            else -> ViewGroup.LayoutParams.WRAP_CONTENT
        }
        val heightSpec = when {
            node.height != null -> dpToPx(context, node.height)
            node.maxHeight is WaterUIWidgetSchema.MaxLength.Infinity -> ViewGroup.LayoutParams.MATCH_PARENT
            else -> ViewGroup.LayoutParams.WRAP_CONTENT
        }
        container.layoutParams = ViewGroup.LayoutParams(widthSpec, heightSpec)
        node.minWidth?.let { container.minimumWidth = dpToPx(context, it) }
        node.minHeight?.let { container.minimumHeight = dpToPx(context, it) }

        val child = render(context, node.content)
        val gravity = resolveGravity(node.alignment, Gravity.CENTER)
        container.addView(
            child,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                gravity,
            ),
        )
        return container
    }

    private fun renderBackground(context: Context, node: WaterUIWidgetSchema.Node.Background): View {
        val container = FrameLayout(context)
        val background = node.background
        if (background is WaterUIWidgetSchema.Node.Color) {
            container.setBackgroundColor(resolveColor(context, background.color))
        } else {
            val backgroundView = render(context, background)
            container.addView(
                backgroundView,
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                ),
            )
        }
        val contentView = render(context, node.content)
        container.addView(
            contentView,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
        return container
    }

    private fun renderAccessibilityLabel(
        context: Context,
        node: WaterUIWidgetSchema.Node.AccessibilityLabel,
    ): View {
        val view = render(context, node.content)
        view.contentDescription = resolveLabelText(context, node.label)
        view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        return view
    }

    private fun renderLink(context: Context, node: WaterUIWidgetSchema.Node.Link): View {
        val view = render(context, node.content)
        view.setOnClickListener {
            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(node.url)).apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            context.startActivity(intent)
        }
        return view
    }

    private fun renderIntent(context: Context, node: WaterUIWidgetSchema.Node.Intent): View {
        val view = render(context, node.label)
        view.setOnClickListener {
            val broadcast = Intent(ACTION_WIDGET_INTENT).apply {
                `package` = context.packageName
                putExtra(EXTRA_INTENT_KIND, node.kind)
                putExtra(EXTRA_INTENT_PAYLOAD, node.payload)
            }
            context.sendBroadcast(broadcast)
        }
        return view
    }

    private fun resolveLabelText(
        context: Context,
        label: WaterUIWidgetSchema.LabelText,
    ): String {
        return when (label) {
            is WaterUIWidgetSchema.LabelText.Localized -> resolveLocalizedString(context, label.key)
            is WaterUIWidgetSchema.LabelText.Verbatim -> label.text
        }
    }

    private fun resolveLocalizedString(context: Context, key: String): String {
        val resId = context.resources.getIdentifier(key, "string", context.packageName)
        return if (resId != 0) context.getString(resId) else key
    }

    private fun applyFont(
        textView: TextView,
        font: WaterUIWidgetSchema.Font?,
        bold: Boolean?,
    ) {
        var sizeSp = 15f
        var weight = 400
        when (font) {
            is WaterUIWidgetSchema.Font.Named -> when (font.style) {
                WaterUIWidgetSchema.FontStyle.LARGE_TITLE -> { sizeSp = 32f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.TITLE -> { sizeSp = 26f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.TITLE2 -> { sizeSp = 22f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.TITLE3 -> { sizeSp = 20f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.HEADLINE -> { sizeSp = 16f; weight = 600 }
                WaterUIWidgetSchema.FontStyle.SUBHEADLINE -> { sizeSp = 14f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.BODY -> { sizeSp = 15f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.CALLOUT -> { sizeSp = 15f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.FOOTNOTE -> { sizeSp = 13f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.CAPTION -> { sizeSp = 12f; weight = 400 }
                WaterUIWidgetSchema.FontStyle.CAPTION2 -> { sizeSp = 11f; weight = 400 }
            }
            is WaterUIWidgetSchema.Font.System -> {
                sizeSp = font.size.toFloat()
                weight = when (font.weight) {
                    WaterUIWidgetSchema.FontWeight.ULTRA_LIGHT -> 100
                    WaterUIWidgetSchema.FontWeight.THIN -> 200
                    WaterUIWidgetSchema.FontWeight.LIGHT -> 300
                    WaterUIWidgetSchema.FontWeight.REGULAR -> 400
                    WaterUIWidgetSchema.FontWeight.MEDIUM -> 500
                    WaterUIWidgetSchema.FontWeight.SEMIBOLD -> 600
                    WaterUIWidgetSchema.FontWeight.BOLD -> 700
                    WaterUIWidgetSchema.FontWeight.HEAVY -> 800
                    WaterUIWidgetSchema.FontWeight.BLACK -> 900
                }
            }
            null -> {}
        }
        if (bold == true && weight < 700) {
            weight = 700
        } else if (bold == false && weight >= 600) {
            weight = 400
        }
        textView.setTextSize(TypedValue.COMPLEX_UNIT_SP, sizeSp)
        textView.typeface = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            Typeface.create(Typeface.DEFAULT, weight, false)
        } else if (weight >= 600) {
            Typeface.DEFAULT_BOLD
        } else {
            Typeface.DEFAULT
        }
    }

    private fun resolveColor(context: Context, color: WaterUIWidgetSchema.WidgetColor): Int {
        return when (color) {
            is WaterUIWidgetSchema.WidgetColor.Srgb -> Color.argb(
                (color.opacity.coerceIn(0f, 1f) * 255f).roundToInt(),
                (color.red.coerceIn(0f, 1f) * 255f).roundToInt(),
                (color.green.coerceIn(0f, 1f) * 255f).roundToInt(),
                (color.blue.coerceIn(0f, 1f) * 255f).roundToInt(),
            )
            is WaterUIWidgetSchema.WidgetColor.Semantic -> {
                val attr = when (color.semantic) {
                    WaterUIWidgetSchema.SemanticColor.PRIMARY -> android.R.attr.textColorPrimary
                    WaterUIWidgetSchema.SemanticColor.SECONDARY -> android.R.attr.textColorSecondary
                    WaterUIWidgetSchema.SemanticColor.ACCENT -> android.R.attr.colorAccent
                }
                val typed = TypedValue()
                if (context.theme.resolveAttribute(attr, typed, true)) {
                    if (typed.resourceId != 0) {
                        context.getColor(typed.resourceId)
                    } else {
                        typed.data
                    }
                } else {
                    when (color.semantic) {
                        WaterUIWidgetSchema.SemanticColor.PRIMARY -> Color.BLACK
                        WaterUIWidgetSchema.SemanticColor.SECONDARY -> Color.DKGRAY
                        WaterUIWidgetSchema.SemanticColor.ACCENT -> Color.BLUE
                    }
                }
            }
        }
    }

    private fun resolveHorizontalGravity(
        alignment: WaterUIWidgetSchema.HorizontalAlignment?,
    ): Int {
        return when (alignment) {
            WaterUIWidgetSchema.HorizontalAlignment.LEADING, null -> Gravity.START
            WaterUIWidgetSchema.HorizontalAlignment.CENTER -> Gravity.CENTER_HORIZONTAL
            WaterUIWidgetSchema.HorizontalAlignment.TRAILING -> Gravity.END
        }
    }

    private fun resolveVerticalGravity(
        alignment: WaterUIWidgetSchema.VerticalAlignment?,
    ): Int {
        return when (alignment) {
            WaterUIWidgetSchema.VerticalAlignment.TOP, null -> Gravity.TOP
            WaterUIWidgetSchema.VerticalAlignment.CENTER -> Gravity.CENTER_VERTICAL
            WaterUIWidgetSchema.VerticalAlignment.BOTTOM -> Gravity.BOTTOM
        }
    }

    private fun resolveGravity(
        alignment: WaterUIWidgetSchema.Alignment?,
        defaultGravity: Int,
    ): Int {
        if (alignment == null) {
            return defaultGravity
        }
        val horizontal = resolveHorizontalGravity(alignment.horizontal)
        val vertical = resolveVerticalGravity(alignment.vertical)
        return horizontal or vertical
    }

    private fun dpToPx(context: Context, dp: Float): Int {
        return dpToPxFloat(context, dp).roundToInt()
    }

    private fun dpToPxFloat(context: Context, dp: Float): Float {
        return TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            dp,
            context.resources.displayMetrics,
        )
    }
}
