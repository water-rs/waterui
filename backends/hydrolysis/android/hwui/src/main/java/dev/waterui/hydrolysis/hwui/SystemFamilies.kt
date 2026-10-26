package dev.waterui.hydrolysis.hwui

import android.graphics.Paint
import android.graphics.Typeface
import android.graphics.text.TextRunShaper
import android.graphics.fonts.Font

/** Platform font matching for system families, strict about unknown names. */
internal object SystemFamilies {
    private const val DEFAULT_FAMILY = "sans-serif"

    /**
     * The system typeface `family` names, or null when the platform has no
     * such family: `Typeface.create` answers an unknown name with
     * [Typeface.DEFAULT] itself, which is told apart by identity.
     */
    fun resolve(family: String): Typeface? {
        val typeface = Typeface.create(family, Typeface.NORMAL)
        return if (typeface === Typeface.DEFAULT && family != DEFAULT_FAMILY) null else typeface
    }

    /** The font the platform's matching draws `family`'s regular text with. */
    fun primaryFont(family: String): Font {
        val typeface = resolve(family) ?: throw IllegalArgumentException("the system has no font family `$family`")
        val paint = Paint()
        paint.typeface = typeface
        val glyphs = TextRunShaper.shapeTextRun("x", 0, 1, 0, 1, 0f, 0f, false, paint)
        if (glyphs.glyphCount() == 0) throw IllegalArgumentException("the system font family `$family` shapes no glyph")
        return glyphs.getFont(0)
    }
}
