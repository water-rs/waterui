package __BUNDLE_IDENTIFIER__

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import dev.waterui.android.runtime.WaterUIApplication

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        System.loadLibrary("__CRATE_NAME_SANITIZED__")

        setContent {
            WaterUIApplication()
        }
    }
}
