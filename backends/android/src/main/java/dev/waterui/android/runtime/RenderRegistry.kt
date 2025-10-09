package dev.waterui.android.runtime

import androidx.compose.runtime.Immutable
import dev.waterui.android.components.*

/**
 * Registry of known view types. Compose components register themselves via [defaultComponents].
 */
@Immutable
class RenderRegistry private constructor(
    private val entries: Map<WuiTypeId, WuiRenderer>
) {
    fun resolve(typeId: WuiTypeId): WuiRenderer? = entries[typeId]

    fun with(typeId: WuiTypeId, renderer: WuiRenderer): RenderRegistry =
        RenderRegistry(entries + (typeId to renderer))

    companion object {
        fun default(): RenderRegistry = RenderRegistry(defaultComponents)
    }
}

/**
 * Simple node descriptor that wraps the native pointer and metadata we receive via JNI.
 */
data class WuiNode(
    val rawPtr: Long,
    val typeId: WuiTypeId
)

/**
 * Populated lazily to avoid referencing components before they are defined. Each component contributes
 * its ID provider via `registerX` functions (see the bottom of the file).
 */
private val defaultComponents: Map<WuiTypeId, WuiRenderer> by lazy {
    buildMap {
        registerWuiEmptyView()
        registerWuiText()
        registerWuiLabel()
        registerWuiButton()
        registerWuiColor()
        registerWuiTextField()
        registerWuiStepper()
        registerWuiProgress()
        registerWuiDynamic()
        registerWuiScroll()
        registerWuiContainer()
        registerWuiToggle()
        registerWuiSpacer()
        registerWuiSlider()
    }
}
