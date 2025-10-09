package dev.waterui.android.layout

import androidx.compose.runtime.Composable
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.MeasurePolicy
import androidx.compose.ui.unit.Constraints
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiEnvironment
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

/**
 * Compose layout that mirrors the Swift `RustLayout` pass-through. All measurement/placement logic is
 * delegated to the Rust layout engine via JNI.
 */
@Composable
fun RustLayout(
    layoutPtr: Long,
    descriptors: List<ChildDescriptor>,
    environment: WuiEnvironment,
    content: @androidx.compose.runtime.Composable () -> Unit
) {
    Layout(
        content = content,
        measurePolicy = layoutMeasurePolicy(layoutPtr, descriptors, environment)
    )
}

private fun layoutMeasurePolicy(
    layoutPtr: Long,
    descriptors: List<ChildDescriptor>,
    environment: WuiEnvironment
): MeasurePolicy = MeasurePolicy { measurables, constraints ->
    // TODO: Implement the three-phase lifecycle using JNI calls (propose → size → place).
    // For now, fallback to Compose intrinsic measurement to keep the skeleton compilable.
    val placeables = measurables.map { it.measure(constraints) }
    val width = placeables.maxOfOrNull { it.width } ?: constraints.minWidth
    val height = placeables.sumOf { it.height }

    layout(width, height) {
        var y = 0
        placeables.forEach { placeable ->
            placeable.placeRelative(0, y)
            y += placeable.height
        }
    }
}

data class ChildDescriptor(
    val typeId: WuiTypeId,
    val isSpacer: Boolean
)

fun LongArray.toChildDescriptors(): List<ChildDescriptor> {
    // TODO: Convert native child metadata to descriptors once JNI exposes it.
    return emptyList()
}
