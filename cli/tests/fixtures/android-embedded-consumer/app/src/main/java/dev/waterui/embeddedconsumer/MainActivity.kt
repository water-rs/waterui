package dev.waterui.embeddedconsumer

import android.os.Bundle
import androidx.activity.ComponentActivity
// `examples/form`'s generated entry point: `<bundle identifier>.waterui`.
import com.waterui.formexample.waterui.WaterUi

/** Mounts the embedded library exactly as `water build` prints it. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(WaterUi.createView(this) { finish() })
    }
}
