package dev.waterui.android.reference

import androidx.compose.runtime.Composable

/** Statically selects this variant's twin — see app/build.gradle.kts. */
fun fixtureContent(example: String): (@Composable () -> Unit)? = { WebviewTwin() }
