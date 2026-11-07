package dev.waterui.hydrolysis.hwui

import android.graphics.RenderEffect
import android.graphics.RuntimeShader
import android.graphics.Shader
import dev.waterui.hydrolysis.OverscrollStretchShader

/**
 * Builds the platform `RenderEffect` chain a `DefineEffect` record describes:
 * `createBlurEffect` stages, one linear-light matrix shader per colour run
 * and the overscroll stretch shader, chained inner-to-outer in application
 * order. Every stage runs on the API floor; the checks here keep a malformed
 * replay honest.
 */
private const val LINEAR_COLOR_SHADER = "dev/waterui/hydrolysis/hwui/linear_color.agsl"

/**
 * The values of `overscroll_stretch.agsl`'s uniforms for one stretch stage:
 * the 12 stage values hold the frame `[a, b, c, d, e, f]` from the stretched
 * viewport's space to the drawn node's own pixels, the viewport's extent,
 * HWUI's stretch direction and the interpolation strength. The frame is
 * already in the node's space, so the shader's window `origin` is zero.
 */
internal class StretchStageUniforms(values: FloatArray, base: Int) {
    val origin = floatArrayOf(0f, 0f)
    val frameLinear = floatArrayOf(values[base], values[base + 1], values[base + 2], values[base + 3])
    val frameOffset = floatArrayOf(values[base + 4], values[base + 5])
    val size = floatArrayOf(values[base + 6], values[base + 7])
    val direction = floatArrayOf(values[base + 8], values[base + 9])
    val strength = values[base + 10]

    fun applyTo(shader: RuntimeShader) {
        shader.setFloatUniform("origin", origin)
        shader.setFloatUniform("frameLinear", frameLinear)
        shader.setFloatUniform("frameOffset", frameOffset)
        shader.setFloatUniform("size", size)
        shader.setFloatUniform("stretch", direction)
        shader.setFloatUniform("strength", strength)
    }
}

internal class NativeEffects {
    private var source: String? = null
    private var stretchSource: String? = null

    /**
     * The stretch shaders each effect draws through, one per stretch stage.
     * A [RenderEffect] captures its shader's uniforms when it is created, so
     * a redefinition rewrites the retained shader and builds a new effect
     * instead of compiling the AGSL again every frame of a pull.
     */
    private val stretchShaders = HashMap<Int, ArrayList<RuntimeShader>>()

    private fun linearShader(): RuntimeShader {
        val loader = checkNotNull(NativeEffects::class.java.classLoader) {
            "HWUI replay: NativeEffects has no class loader to read its AGSL resource"
        }
        val cached = source ?: loader
            .getResourceAsStream(LINEAR_COLOR_SHADER)
            ?.use { it.readBytes().toString(Charsets.UTF_8) }
            ?: throw CommandBufferException("HWUI replay: the linear-color AGSL resource is missing")
        source = cached
        return RuntimeShader(cached)
    }

    private fun stretchShader(effect: Int, stage: Int): RuntimeShader {
        val shaders = stretchShaders.getOrPut(effect) { ArrayList() }
        if (stage == shaders.size) {
            val cached = stretchSource ?: OverscrollStretchShader.source().also { stretchSource = it }
            shaders.add(RuntimeShader(cached))
        }
        return shaders[stage]
    }

    /** Forgets the shaders `effect` retained; the effect is released. */
    fun release(effect: Int) {
        stretchShaders.remove(effect)
    }

    /** Forgets every retained shader. */
    fun clear() = stretchShaders.clear()

    fun build(effect: Int, count: Int, kinds: IntArray, values: FloatArray): RenderEffect {
        var chain: RenderEffect? = null
        var stretches = 0
        for (i in 0 until count) {
            val stage: RenderEffect? = when (kinds[i]) {
                EffectKind.BLUR -> {
                    val radius = values[i * 12]
                    if (radius > 0f) {
                        RenderEffect.createBlurEffect(radius, radius, Shader.TileMode.CLAMP)
                    } else {
                        null
                    }
                }
                EffectKind.LINEAR_COLOR -> {
                    val shader = linearShader()
                    val base = i * 12
                    shader.setFloatUniform("row0", values[base], values[base + 1], values[base + 2], values[base + 3])
                    shader.setFloatUniform("row1", values[base + 4], values[base + 5], values[base + 6], values[base + 7])
                    shader.setFloatUniform("row2", values[base + 8], values[base + 9], values[base + 10], values[base + 11])
                    RenderEffect.createRuntimeShaderEffect(shader, "inputImage")
                }
                EffectKind.OVERSCROLL_STRETCH -> {
                    val shader = stretchShader(effect, stretches++)
                    StretchStageUniforms(values, i * 12).applyTo(shader)
                    RenderEffect.createRuntimeShaderEffect(shader, "content")
                }
                else -> throw CommandBufferException("HWUI replay: effect $effect has unknown stage kind ${kinds[i]}")
            }
            if (stage != null) {
                chain = if (chain == null) stage else RenderEffect.createChainEffect(stage, chain)
            }
        }
        return chain ?: RenderEffect.createOffsetEffect(0f, 0f)
    }
}
