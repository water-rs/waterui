package dev.waterui.android.bench

import android.content.Context
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject

/** Load the shared interaction spec bundled into the test APK's assets. */
fun loadInteractionSpec(context: Context): JSONObject =
    Journey.loadSpec(
        context.assets.open("interactions.json").bufferedReader().use {
            it.readText()
        }
    )

/** The spec's environment map for a fixture (suite.toml defaults + env). */
fun specEnv(spec: JSONObject, fixture: String): JSONObject {
    val fixtures = spec.getJSONObject("fixtures")
    if (!fixtures.has(fixture)) return JSONObject()
    return fixtures.getJSONObject(fixture).optJSONObject("env") ?: JSONObject()
}
