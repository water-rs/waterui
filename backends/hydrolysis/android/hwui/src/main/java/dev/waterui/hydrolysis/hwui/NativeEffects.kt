package dev.waterui.hydrolysis.hwui

import android.graphics.RenderEffect
import android.graphics.RuntimeShader
import android.graphics.Shader
import android.os.Build
import androidx.annotation.RequiresApi

/**
 * Builds the platform `RenderEffect` chain a `DefineEffect` record describes:
 * `createBlurEffect` stages plus one linear-light matrix shader per colour
 * run, chained inner-to-outer in application order. Registration on the Rust
 * side already rejects a chain the device's API level cannot run; the checks
 * here keep a malformed replay honest rather than second-guessing it.
 */
private const val LINEAR_COLOR_SHADER = "dev/waterui/hydrolysis/hwui/linear_color.agsl"

internal class NativeEffects {
    private var source: String? = null

    @RequiresApi(33)
    private fun linearShader(): RuntimeShader {
        val cached = source ?: NativeEffects::class.java.classLoader
            .getResourceAsStream(LINEAR_COLOR_SHADER)
            ?.use { it.readBytes().toString(Charsets.UTF_8) }
            ?: throw CommandBufferException("HWUI replay: the linear-color AGSL resource is missing")
        source = cached
        return RuntimeShader(cached)
    }

    fun build(effect: Int, count: Int, kinds: IntArray, values: FloatArray): RenderEffect {
        if (Build.VERSION.SDK_INT < 33 && (0 until count).any { kinds[it] == EffectKind.LINEAR_COLOR }) {
            throw CommandBufferException("HWUI replay: colour effect $effect needs API 33")
        }
        var chain: RenderEffect? = null
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
                else -> throw CommandBufferException("HWUI replay: effect $effect has unknown stage kind ${kinds[i]}")
            }
            if (stage != null) {
                chain = if (chain == null) stage else RenderEffect.createChainEffect(stage, chain)
            }
        }
        return chain ?: RenderEffect.createOffsetEffect(0f, 0f)
    }
}
