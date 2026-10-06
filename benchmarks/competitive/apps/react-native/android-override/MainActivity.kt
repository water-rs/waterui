package com.rnbench

import android.os.Bundle
import com.facebook.react.ReactActivity
import com.facebook.react.ReactActivityDelegate
import com.facebook.react.defaults.DefaultReactActivityDelegate

/** Harness-authored override of the generated template activity: launch
 *  options forward the workload/step intent extras to the JS root
 *  component as initial props, the same convention the iOS AppDelegate
 *  implements with `-bench-*` launch arguments. Values are
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
        val workload = host.intent?.getStringExtra("workload")
        workload?.let { putString("workload", it) }
        // This leg measures one ladder step per launch: W5/W6 without
        // `step` traps — a silently self-paced ladder would produce a
        // different measurement than every other contestant.
        if (workload != null && workload.lowercase() in setOf("w5", "w6")
            && host.intent?.hasExtra("step") != true) {
          error("$workload requires a `step` intent extra on this leg")
        }
        if (host.intent?.hasExtra("step") == true) {
          putInt("step", host.intent!!.getIntExtra("step", 0))
        }
      }
}

class MainActivity : ReactActivity() {

  override fun getMainComponentName(): String = "RnBench"

  override fun createReactActivityDelegate(): ReactActivityDelegate =
      BenchDelegate(this, mainComponentName)
}
