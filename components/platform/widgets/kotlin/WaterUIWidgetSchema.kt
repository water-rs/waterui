package dev.waterui.widgets

import org.json.JSONArray
import org.json.JSONObject

/**
 * Kotlin mirror of `components/platform/widgets/src/schema.rs`.
 *
 * Decodes timeline snapshots produced by `#[waterui::widget]` functions using
 * Android's built-in `org.json` API so no third-party serialization dependency
 * is required on the Android widget classpath.
 */
object WaterUIWidgetSchema {
    data class Timeline(
        val entries: List<TimelineEntry>,
        val policy: ReloadPolicy,
    )

    data class TimelineEntry(
        val epochSeconds: Double,
        val content: Node,
    )

    sealed class ReloadPolicy {
        data object AtEnd : ReloadPolicy()
        data object Never : ReloadPolicy()
        data class After(val epochSeconds: Double) : ReloadPolicy()
    }

    sealed class LabelText {
        data class Localized(val key: String) : LabelText()
        data class Verbatim(val text: String) : LabelText()
    }

    sealed class TextContent {
        data class Localized(val key: String) : TextContent()
        data class Verbatim(val text: String) : TextContent()
        data class Date(val epochSeconds: Double, val style: DateStyle) : TextContent()
    }

    enum class DateStyle {
        RELATIVE,
        OFFSET,
        TIMER,
        DATE,
        TIME,
    }

    sealed class Font {
        data class Named(val style: FontStyle) : Font()
        data class System(val size: Double, val weight: FontWeight) : Font()
    }

    enum class FontStyle {
        LARGE_TITLE,
        TITLE,
        TITLE2,
        TITLE3,
        HEADLINE,
        SUBHEADLINE,
        BODY,
        CALLOUT,
        FOOTNOTE,
        CAPTION,
        CAPTION2,
    }

    enum class FontWeight {
        ULTRA_LIGHT,
        THIN,
        LIGHT,
        REGULAR,
        MEDIUM,
        SEMIBOLD,
        BOLD,
        HEAVY,
        BLACK,
    }

    sealed class WidgetColor {
        data class Srgb(
            val red: Float,
            val green: Float,
            val blue: Float,
            val opacity: Float,
        ) : WidgetColor()

        data class Semantic(val semantic: SemanticColor) : WidgetColor()
    }

    enum class SemanticColor {
        PRIMARY,
        SECONDARY,
        ACCENT,
    }

    sealed class ImageSource {
        data class SfSymbol(val name: String) : ImageSource()
        data class Data(val bytes: ByteArray) : ImageSource()
    }

    enum class Axis {
        HORIZONTAL,
        VERTICAL,
        DEPTH,
    }

    enum class HorizontalAlignment {
        LEADING,
        CENTER,
        TRAILING,
    }

    enum class VerticalAlignment {
        TOP,
        CENTER,
        BOTTOM,
    }

    data class Alignment(
        val horizontal: HorizontalAlignment?,
        val vertical: VerticalAlignment?,
    )

    data class EdgeInsets(
        val top: Float,
        val bottom: Float,
        val leading: Float,
        val trailing: Float,
    )

    sealed class MaxLength {
        data class Fixed(val value: Float) : MaxLength()
        data object Infinity : MaxLength()
    }

    sealed class ShapeKind {
        data object Rectangle : ShapeKind()
        data class RoundedRectangle(val cornerRadius: Float) : ShapeKind()
        data object Capsule : ShapeKind()
        data object Circle : ShapeKind()
        data object Ellipse : ShapeKind()
    }

    sealed class Node {
        data class Text(
            val content: TextContent,
            val font: Font?,
            val bold: Boolean?,
            val color: WidgetColor?,
            val lineLimit: Int?,
        ) : Node()

        data class Image(
            val source: ImageSource,
            val color: WidgetColor?,
        ) : Node()

        data class Stack(
            val axis: Axis,
            val children: List<Node>,
            val spacing: Float?,
            val alignment: Alignment?,
        ) : Node()

        data class Shape(
            val kind: ShapeKind,
            val fill: WidgetColor,
        ) : Node()

        data class Color(
            val color: WidgetColor,
        ) : Node()

        data class Padding(
            val insets: EdgeInsets?,
            val content: Node,
        ) : Node()

        data class Spacer(
            val minLength: Float?,
        ) : Node()

        data class Frame(
            val width: Float?,
            val height: Float?,
            val minWidth: Float?,
            val maxWidth: MaxLength?,
            val minHeight: Float?,
            val maxHeight: MaxLength?,
            val alignment: Alignment?,
            val content: Node,
        ) : Node()

        data class Background(
            val content: Node,
            val background: Node,
        ) : Node()

        data class AccessibilityLabel(
            val label: LabelText,
            val content: Node,
        ) : Node()

        data class Link(
            val url: String,
            val content: Node,
        ) : Node()

        data class Intent(
            val kind: String,
            val payload: String,
            val label: Node,
        ) : Node()
    }

    @JvmStatic
    fun decodeTimeline(json: String): Timeline {
        val root = JSONObject(json)
        val entriesJson = root.getJSONArray("entries")
        val entries = ArrayList<TimelineEntry>(entriesJson.length())
        for (i in 0 until entriesJson.length()) {
            val item = entriesJson.getJSONObject(i)
            entries.add(
                TimelineEntry(
                    epochSeconds = item.getDouble("date"),
                    content = decodeNode(item.getJSONObject("content")),
                ),
            )
        }
        val policy = decodeReloadPolicy(root.getJSONObject("policy"))
        return Timeline(entries = entries, policy = policy)
    }

    @JvmStatic
    fun decodeNode(json: String): Node = decodeNode(JSONObject(json))

    private fun decodeReloadPolicy(obj: JSONObject): ReloadPolicy {
        return when (val type = obj.getString("type")) {
            "atEnd" -> ReloadPolicy.AtEnd
            "never" -> ReloadPolicy.Never
            "after" -> ReloadPolicy.After(obj.getDouble("date"))
            else -> throw IllegalArgumentException("unknown reload policy: $type")
        }
    }

    private fun decodeNode(obj: JSONObject): Node {
        return when (val type = obj.getString("type")) {
            "text" -> Node.Text(
                content = decodeTextContent(obj.getJSONObject("content")),
                font = obj.optJSONObject("font")?.let(::decodeFont),
                bold = if (obj.has("bold") && !obj.isNull("bold")) obj.getBoolean("bold") else null,
                color = obj.optJSONObject("color")?.let(::decodeColor),
                lineLimit = if (obj.has("lineLimit") && !obj.isNull("lineLimit")) {
                    obj.getInt("lineLimit")
                } else {
                    null
                },
            )
            "image" -> Node.Image(
                source = decodeImageSource(obj.getJSONObject("source")),
                color = obj.optJSONObject("color")?.let(::decodeColor),
            )
            "stack" -> {
                val childrenJson = obj.getJSONArray("children")
                val children = ArrayList<Node>(childrenJson.length())
                for (i in 0 until childrenJson.length()) {
                    children.add(decodeNode(childrenJson.getJSONObject(i)))
                }
                Node.Stack(
                    axis = decodeAxis(obj.getString("axis")),
                    children = children,
                    spacing = optFloat(obj, "spacing"),
                    alignment = obj.optJSONObject("alignment")?.let(::decodeAlignment),
                )
            }
            "shape" -> Node.Shape(
                kind = decodeShapeKind(obj.getJSONObject("kind")),
                fill = decodeColor(obj.getJSONObject("fill")),
            )
            "color" -> Node.Color(decodeColor(obj))
            "padding" -> Node.Padding(
                insets = obj.optJSONObject("insets")?.let(::decodeEdgeInsets),
                content = decodeNode(obj.getJSONObject("content")),
            )
            "spacer" -> Node.Spacer(minLength = optFloat(obj, "minLength"))
            "frame" -> Node.Frame(
                width = optFloat(obj, "width"),
                height = optFloat(obj, "height"),
                minWidth = optFloat(obj, "minWidth"),
                maxWidth = obj.optJSONObject("maxWidth")?.let(::decodeMaxLength),
                minHeight = optFloat(obj, "minHeight"),
                maxHeight = obj.optJSONObject("maxHeight")?.let(::decodeMaxLength),
                alignment = obj.optJSONObject("alignment")?.let(::decodeAlignment),
                content = decodeNode(obj.getJSONObject("content")),
            )
            "background" -> Node.Background(
                content = decodeNode(obj.getJSONObject("content")),
                background = decodeNode(obj.getJSONObject("background")),
            )
            "accessibilityLabel" -> Node.AccessibilityLabel(
                label = decodeLabelText(obj.getJSONObject("label")),
                content = decodeNode(obj.getJSONObject("content")),
            )
            "link" -> Node.Link(
                url = obj.getString("url"),
                content = decodeNode(obj.getJSONObject("content")),
            )
            "intent" -> Node.Intent(
                kind = obj.getString("kind"),
                payload = obj.getString("payload"),
                label = decodeNode(obj.getJSONObject("label")),
            )
            else -> throw IllegalArgumentException("unknown widget node type: $type")
        }
    }

    private fun decodeLabelText(obj: JSONObject): LabelText {
        return when (val kind = obj.getString("kind")) {
            "localized" -> LabelText.Localized(obj.getString("key"))
            "verbatim" -> LabelText.Verbatim(obj.getString("text"))
            else -> throw IllegalArgumentException("unknown label text kind: $kind")
        }
    }

    private fun decodeTextContent(obj: JSONObject): TextContent {
        return when (val kind = obj.getString("kind")) {
            "localized" -> TextContent.Localized(obj.getString("key"))
            "verbatim" -> TextContent.Verbatim(obj.getString("text"))
            "date" -> TextContent.Date(
                epochSeconds = obj.getDouble("date"),
                style = decodeDateStyle(obj.getString("style")),
            )
            else -> throw IllegalArgumentException("unknown text content kind: $kind")
        }
    }

    private fun decodeDateStyle(raw: String): DateStyle {
        return when (raw) {
            "relative" -> DateStyle.RELATIVE
            "offset" -> DateStyle.OFFSET
            "timer" -> DateStyle.TIMER
            "date" -> DateStyle.DATE
            "time" -> DateStyle.TIME
            else -> throw IllegalArgumentException("unknown date style: $raw")
        }
    }

    private fun decodeFont(obj: JSONObject): Font {
        return when (val kind = obj.getString("kind")) {
            "named" -> Font.Named(decodeFontStyle(obj.getString("style")))
            "system" -> Font.System(
                size = obj.getDouble("size"),
                weight = decodeFontWeight(obj.getString("weight")),
            )
            else -> throw IllegalArgumentException("unknown font kind: $kind")
        }
    }

    private fun decodeFontStyle(raw: String): FontStyle {
        return when (raw) {
            "largeTitle" -> FontStyle.LARGE_TITLE
            "title" -> FontStyle.TITLE
            "title2" -> FontStyle.TITLE2
            "title3" -> FontStyle.TITLE3
            "headline" -> FontStyle.HEADLINE
            "subheadline" -> FontStyle.SUBHEADLINE
            "body" -> FontStyle.BODY
            "callout" -> FontStyle.CALLOUT
            "footnote" -> FontStyle.FOOTNOTE
            "caption" -> FontStyle.CAPTION
            "caption2" -> FontStyle.CAPTION2
            else -> throw IllegalArgumentException("unknown font style: $raw")
        }
    }

    private fun decodeFontWeight(raw: String): FontWeight {
        return when (raw) {
            "ultraLight" -> FontWeight.ULTRA_LIGHT
            "thin" -> FontWeight.THIN
            "light" -> FontWeight.LIGHT
            "regular" -> FontWeight.REGULAR
            "medium" -> FontWeight.MEDIUM
            "semibold" -> FontWeight.SEMIBOLD
            "bold" -> FontWeight.BOLD
            "heavy" -> FontWeight.HEAVY
            "black" -> FontWeight.BLACK
            else -> throw IllegalArgumentException("unknown font weight: $raw")
        }
    }

    private fun decodeColor(obj: JSONObject): WidgetColor {
        return when (val kind = obj.getString("kind")) {
            "srgb" -> WidgetColor.Srgb(
                red = obj.getDouble("red").toFloat(),
                green = obj.getDouble("green").toFloat(),
                blue = obj.getDouble("blue").toFloat(),
                opacity = obj.getDouble("opacity").toFloat(),
            )
            "semantic" -> WidgetColor.Semantic(decodeSemanticColor(obj.getString("semantic")))
            else -> throw IllegalArgumentException("unknown color kind: $kind")
        }
    }

    private fun decodeSemanticColor(raw: String): SemanticColor {
        return when (raw) {
            "primary" -> SemanticColor.PRIMARY
            "secondary" -> SemanticColor.SECONDARY
            "accent" -> SemanticColor.ACCENT
            else -> throw IllegalArgumentException("unknown semantic color: $raw")
        }
    }

    private fun decodeImageSource(obj: JSONObject): ImageSource {
        return when (val kind = obj.getString("kind")) {
            "sfSymbol" -> ImageSource.SfSymbol(obj.getString("name"))
            "data" -> ImageSource.Data(decodeByteArray(obj.getJSONArray("data")))
            else -> throw IllegalArgumentException("unknown image source kind: $kind")
        }
    }

    private fun decodeByteArray(array: JSONArray): ByteArray {
        val bytes = ByteArray(array.length())
        for (i in 0 until array.length()) {
            bytes[i] = array.getInt(i).toByte()
        }
        return bytes
    }

    private fun decodeAxis(raw: String): Axis {
        return when (raw) {
            "horizontal" -> Axis.HORIZONTAL
            "vertical" -> Axis.VERTICAL
            "depth" -> Axis.DEPTH
            else -> throw IllegalArgumentException("unknown stack axis: $raw")
        }
    }

    private fun decodeAlignment(obj: JSONObject): Alignment {
        val horizontal = if (obj.has("horizontal") && !obj.isNull("horizontal")) {
            when (val raw = obj.getString("horizontal")) {
                "leading" -> HorizontalAlignment.LEADING
                "center" -> HorizontalAlignment.CENTER
                "trailing" -> HorizontalAlignment.TRAILING
                else -> throw IllegalArgumentException("unknown horizontal alignment: $raw")
            }
        } else {
            null
        }
        val vertical = if (obj.has("vertical") && !obj.isNull("vertical")) {
            when (val raw = obj.getString("vertical")) {
                "top" -> VerticalAlignment.TOP
                "center" -> VerticalAlignment.CENTER
                "bottom" -> VerticalAlignment.BOTTOM
                else -> throw IllegalArgumentException("unknown vertical alignment: $raw")
            }
        } else {
            null
        }
        return Alignment(horizontal = horizontal, vertical = vertical)
    }

    private fun decodeEdgeInsets(obj: JSONObject): EdgeInsets {
        return EdgeInsets(
            top = obj.getDouble("top").toFloat(),
            bottom = obj.getDouble("bottom").toFloat(),
            leading = obj.getDouble("leading").toFloat(),
            trailing = obj.getDouble("trailing").toFloat(),
        )
    }

    private fun decodeMaxLength(obj: JSONObject): MaxLength {
        return when (val kind = obj.getString("kind")) {
            "fixed" -> MaxLength.Fixed(obj.getDouble("value").toFloat())
            "infinity" -> MaxLength.Infinity
            else -> throw IllegalArgumentException("unknown max length kind: $kind")
        }
    }

    private fun decodeShapeKind(obj: JSONObject): ShapeKind {
        return when (val kind = obj.getString("kind")) {
            "rectangle" -> ShapeKind.Rectangle
            "roundedRectangle" -> ShapeKind.RoundedRectangle(obj.getDouble("cornerRadius").toFloat())
            "capsule" -> ShapeKind.Capsule
            "circle" -> ShapeKind.Circle
            "ellipse" -> ShapeKind.Ellipse
            else -> throw IllegalArgumentException("unknown shape kind: $kind")
        }
    }

    private fun optFloat(obj: JSONObject, key: String): Float? {
        return if (obj.has(key) && !obj.isNull(key)) {
            obj.getDouble(key).toFloat()
        } else {
            null
        }
    }
}
