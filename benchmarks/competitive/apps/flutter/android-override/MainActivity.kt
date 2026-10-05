package dev.bench.bench_flutter

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * Serves the shared `bench/config` MethodChannel from the launch intent's
 * extras — the Android counterpart of the Apple legs' NSArgumentDomain
 * reads. `workload`/`drive` arrive as strings, `step` as an int; absent
 * extras pass through as null and the Dart side traps — never a default.
 */
class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "bench/config")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "workload" -> result.success(intent.getStringExtra("workload"))
                    "drive" -> result.success(intent.getStringExtra("drive"))
                    "step" -> {
                        val step =
                            if (intent.hasExtra("step")) intent.getIntExtra("step", 0)
                            else null
                        result.success(step?.toString())
                    }
                    // Darwin-notify handshake methods exist only on Apple
                    // targets; on Android the runner drives externally.
                    "beginObserved" -> result.success(false)
                    "postDone", "logStep", "discardBegins" -> result.success(null)
                    else -> result.notImplemented()
                }
            }
    }
}
