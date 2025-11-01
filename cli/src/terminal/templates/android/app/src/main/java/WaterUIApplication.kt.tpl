package __BUNDLE_IDENTIFIER__

import android.app.Application
import dev.waterui.android.runtime.configureWaterUiNativeLibrary

class WaterUIApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        configureWaterUiNativeLibrary("__CRATE_NAME_SANITIZED__")
    }
}
