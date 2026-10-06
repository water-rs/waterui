package dev.bench.bench_flutter

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * Serves the shared `bench/config` MethodChannel from the launch intent's
 * extras — the Android counterpart of the Apple legs' NSArgumentDomain
 * reads. `workload` arrives as a string, `step` as an int; absent extras
 * pass through as null and the Dart side traps — never a default.
 */
class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "bench/config")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "workload" -> result.success(intent.getStringExtra("workload"))
                    "step" -> {
                        // This leg measures one ladder step per launch:
                        // W5/W6 without `step` traps — a silently
                        // self-paced ladder would produce a different
                        // measurement than every other contestant.
                        val wl = intent.getStringExtra("workload")
                        if (wl != null && wl.lowercase() in setOf("w5", "w6")
                            && !intent.hasExtra("step")) {
                            result.error("bench", "$wl requires a `step` intent extra on this leg", null)
                            return@setMethodCallHandler
                        }
                        val step =
                            if (intent.hasExtra("step")) intent.getIntExtra("step", 0)
                            else null
                        result.success(step?.toString())
                    }
                    else -> result.notImplemented()
                }
            }
    }
}
