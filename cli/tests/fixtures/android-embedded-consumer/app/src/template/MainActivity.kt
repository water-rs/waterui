package dev.waterui.embeddedconsumer

import android.os.Bundle
import androidx.activity.ComponentActivity
import ${wateruiPackage}.waterui.WaterUi

/** Mounts the embedded library exactly as `water build` prints it. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(WaterUi.createView(this) { finish() })
    }
}
