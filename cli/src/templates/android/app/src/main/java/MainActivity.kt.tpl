package __BUNDLE_IDENTIFIER__

import androidx.appcompat.app.AppCompatActivity
import android.os.Bundle
// Assuming the backend is in this package
import com.waterui.android.App 

class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Load the Rust library
        System.loadLibrary("__CRATE_NAME__")
        
        // Set the content view to the WaterUI root view
        setContentView(App(this))
    }
}
