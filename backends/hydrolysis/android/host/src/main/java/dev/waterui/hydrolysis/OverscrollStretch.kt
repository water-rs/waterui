package dev.waterui.hydrolysis

import android.graphics.RenderEffect
import android.graphics.RuntimeShader
import android.view.View
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/**
 * One scroll view's overscroll stretch as the engine published it with a
 * platform-view placement: the stretch filter's own parameters in
 * `OverscrollStretch`'s flattened order — the frame `[a, b, c, d, e, f]`
 * from the viewport's own space to window logical units, the viewport's
 * extent, HWUI's stretch direction — and HWUI's interpolation strength.
 */
internal class OverscrollStretch(val params: FloatArray, val strength: Float) {
    init {
        require(params.size == PARAMS) {
            "hydrolysis: an overscroll stretch carries $PARAMS parameters, got ${params.size}"
        }
    }

    companion object {
        const val PARAMS = 10
    }
}

/**
 * The values of `overscroll_stretch.agsl`'s uniforms for one stretch on a
 * View whose top-left corner sits at (`left`, `top`) window pixels: the
 * frame carried from window logical units to pixels by `density`, the
 * viewport's extent and the direction unchanged.
 */
internal class StretchUniforms(
    stretch: OverscrollStretch,
    density: Float,
    left: Int,
    top: Int,
) {
    val origin = floatArrayOf(left.toFloat(), top.toFloat())
    val frameLinear: FloatArray
    val frameOffset: FloatArray
    val size: FloatArray
    val direction: FloatArray
    val strength = stretch.strength

    init {
        val p = stretch.params
        frameLinear = floatArrayOf(p[0] * density, p[1] * density, p[2] * density, p[3] * density)
        frameOffset = floatArrayOf(p[4] * density, p[5] * density)
        size = floatArrayOf(p[6], p[7])
        direction = floatArrayOf(p[8], p[9])
    }
}

/**
 * Where a stretched platform view is laid out, in window pixels. The warp
 * moves a View's content past its frame — most of the viewport can show it
 * when the View fills only part of it — and a RenderEffect cannot draw
 * outside the bounds of the View it is set on. The slot therefore takes the
 * region the engine published (the frame joined with the stretching
 * viewports, inside the paint chain's clip) while a stretch shows, and the
 * mounted view keeps its frame size and window position inside it, so the
 * View is never resized or moved by a stretch.
 */
internal class StretchLayout(
    val regionLeft: Int,
    val regionTop: Int,
    val regionWidth: Int,
    val regionHeight: Int,
    val mountedLeft: Int,
    val mountedTop: Int,
    val mountedWidth: Int,
    val mountedHeight: Int,
) {
    companion object {
        /**
         * The layout of the frame `[x, y, width, height]` inside `region`
         * (window logical units, scaled by `density`). The pixel region
         * always encloses the pixel frame the unstretched layout places.
         */
        fun of(frame: FloatArray, region: FloatArray, density: Float): StretchLayout {
            val left = (frame[0] * density).toInt()
            val top = (frame[1] * density).toInt()
            val width = (frame[2] * density).toInt()
            val height = (frame[3] * density).toInt()
            val regionLeft = min(floor(region[0] * density).toInt(), left)
            val regionTop = min(floor(region[1] * density).toInt(), top)
            val regionRight = max(ceil((region[0] + region[2]) * density).toInt(), left + width)
            val regionBottom = max(ceil((region[1] + region[3]) * density).toInt(), top + height)
            return StretchLayout(
                regionLeft,
                regionTop,
                regionRight - regionLeft,
                regionBottom - regionTop,
                left - regionLeft,
                top - regionTop,
                width,
                height,
            )
        }
    }
}

/**
 * The AGSL stretch a platform view's slot draws through while its scroll
 * views overscroll: the engine's stretch stage ported to a
 * [RuntimeShader] and set as the slot's [RenderEffect], one shader per
 * stretching scroll view. Chained innermost first: the inner scroll
 * view's warp reads the content, the outer one's reads the inner's
 * result, the way the engine nests the stretched layers.
 *
 * A [RenderEffect] captures its shader's uniforms when it is created, so
 * every update builds a new effect over the retained shaders.
 */
internal class OverscrollStretchEffect(private val source: String) {
    private val shaders = ArrayList<RuntimeShader>()

    /**
     * Sets the effect for `stretches` (outermost first) on `view`, whose
     * top-left corner is at (`left`, `top`) window pixels.
     */
    fun update(
        view: View,
        stretches: List<OverscrollStretch>,
        density: Float,
        left: Int,
        top: Int,
    ) {
        while (shaders.size > stretches.size) shaders.removeAt(shaders.lastIndex)
        while (shaders.size < stretches.size) shaders.add(RuntimeShader(source))
        val effects =
            stretches.mapIndexed { index, stretch ->
                val uniforms = StretchUniforms(stretch, density, left, top)
                val shader = shaders[index]
                shader.setFloatUniform("origin", uniforms.origin)
                shader.setFloatUniform("frameLinear", uniforms.frameLinear)
                shader.setFloatUniform("frameOffset", uniforms.frameOffset)
                shader.setFloatUniform("size", uniforms.size)
                shader.setFloatUniform("stretch", uniforms.direction)
                shader.setFloatUniform("strength", uniforms.strength)
                RenderEffect.createRuntimeShaderEffect(shader, "content")
            }
        view.setRenderEffect(
            effects.reduceRight { outer, inner -> RenderEffect.createChainEffect(outer, inner) },
        )
    }

    /** Draws `view` unstretched again. */
    fun clear(view: View) {
        shaders.clear()
        view.setRenderEffect(null)
    }

}

/**
 * The AGSL source of the engine's stretch stage, the one copy both the
 * platform-view warp and the HWUI painter's `RenderEffect` run: a Java
 * resource of this library.
 */
object OverscrollStretchShader {
    private const val RESOURCE = "hydrolysis/overscroll_stretch.agsl"

    /** The source text; read on every call, so callers keep the result. */
    fun source(): String {
        val loader =
            checkNotNull(OverscrollStretchShader::class.java.classLoader) {
                "hydrolysis: no class loader to read the overscroll stretch AGSL resource"
            }
        val stream =
            checkNotNull(loader.getResourceAsStream(RESOURCE)) {
                "hydrolysis: the overscroll stretch AGSL resource $RESOURCE is missing"
            }
        return stream.use { it.readBytes().toString(Charsets.UTF_8) }
    }
}
