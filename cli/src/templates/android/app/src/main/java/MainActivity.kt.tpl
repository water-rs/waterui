package __BUNDLE_IDENTIFIER__

import android.os.Bundle
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity

class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        System.loadLibrary("__CRATE_NAME_SANITIZED__")

        val container = FrameLayout(this)
        setContentView(container)
    }
}
