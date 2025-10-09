package dev.waterui.android.runtime

/**
 * Kotlin representation of `WuiTypeId` (`[u64; 2]` on the Rust side).
 */
data class WuiTypeId(
    val high: Long,
    val low: Long
) {
    companion object {
        val ANY_VIEW: WuiTypeId = WuiTypeId(-1L, -1L) // TODO replace with real ID via JNI helper
    }
}

fun LongArray.toTypeId(): WuiTypeId {
    require(size == 2) { "TypeId arrays must contain exactly two elements." }
    return WuiTypeId(get(0), get(1))
}
