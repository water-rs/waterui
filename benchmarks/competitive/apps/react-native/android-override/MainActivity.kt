package com.rnbench

import android.os.Bundle
import com.facebook.react.ReactActivity
import com.facebook.react.ReactActivityDelegate
import com.facebook.react.defaults.DefaultReactActivityDelegate

/** Harness-authored override of the generated template activity: launch
 *  options forward the workload/drive/step intent extras to the JS root
 *  component as initial props, the same convention the iOS and macOS
 *  AppDelegates implement with `-bench-*` launch arguments. Values are
 *  forwarded verbatim — App.jsx traps on a missing or unrecognized
 *  workload, so a bare launch fails loudly instead of measuring W1.
 *  `intent` is read lazily because it is not set when
 *  createReactActivityDelegate runs. */
private class BenchDelegate(
    private val host: ReactActivity,
    mainComponentName: String,
) : DefaultReactActivityDelegate(host, mainComponentName) {
  override fun getLaunchOptions(): Bundle =
      Bundle().apply {
        host.intent?.getStringExtra("workload")?.let { putString("workload", it) }
        host.intent?.getStringExtra("drive")?.let { putString("drive", it) }
        putInt("step", host.intent?.getIntExtra("step", 0) ?: 0)
      }
}

class MainActivity : ReactActivity() {

  override fun getMainComponentName(): String = "RnBench"

  override fun createReactActivityDelegate(): ReactActivityDelegate =
      BenchDelegate(this, mainComponentName)
}
