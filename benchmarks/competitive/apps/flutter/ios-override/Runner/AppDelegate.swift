import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    // Launch arguments `-bench-workload W2` land in
    // NSUserDefaults' NSArgumentDomain. Missing or unrecognized workload
    // traps — a wrong page must fail, never silently measure W1.
    let rawWorkload = UserDefaults.standard.string(forKey: "bench-workload")
    guard let workload = rawWorkload,
      ["W1", "W2", "W3", "W4", "W5", "W6"].contains(workload)
    else {
      fatalError(
        "missing or unrecognized -bench-workload launch argument "
          + "(got \(rawWorkload ?? "nil")); expected W1..=W6")
    }

    // The runner waits for `dev.bench.ready.<bundle-id>.<W>` to confirm
    // this argument arrived — a deep AX query on the 10k-row feed stalls
    // for minutes, so notify carries the assertion.
    notify_post("dev.bench.ready.\(Bundle.main.bundleIdentifier ?? "unknown").\(workload)")

    // The runner asserts this accessibility identifier after launch;
    // retry until the Flutter view exists.
    for delay in [0.0, 0.5, 1.0, 2.0] {
      DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
        for scene in UIApplication.shared.connectedScenes {
          guard let ws = scene as? UIWindowScene else { continue }
          for window in ws.windows {
            window.rootViewController?.view.accessibilityIdentifier =
              "bench-workload-\(workload)"
          }
        }
      }
    }

    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)

    // Launch arguments `-bench-workload W2` land in NSUserDefaults'
    // NSArgumentDomain; expose them to Dart for workload selection.
    let channel = FlutterMethodChannel(
      name: "bench/config",
      binaryMessenger: engineBridge.applicationRegistrar.messenger())

    // `auto` drive handshake over Darwin notifications — the same mechanism
    // every contestant uses. AX queries cannot carry it: a workload can
    // stall the accessibility server for tens of seconds while it
    // materializes, and a timed-out query fails the test instead of
    // driving it. Dart polls `beginObserved`; when it turns true the app
    // runs the fling program and calls `postDone`.
    // The token stays armed for the whole run: each `begin` post re-fires it,
    // so every XCTest measure iteration re-runs the program. Posts consumed
    // while Dart is mid-program queue in `pending` so none are lost.
    var pending = 0
    var beginToken: Int32 = 0
    if notify_register_check("dev.bench.begin", &beginToken) == UInt32(NOTIFY_STATUS_OK) {
      // The first notify_check reports the flag's CURRENT latched state —
      // consume it so only a post made after registration counts as begin.
      var fired: Int32 = 0
      notify_check(beginToken, &fired)
      func poll() {
        var f: Int32 = 0
        notify_check(beginToken, &f)
        if f != 0 {
          pending += 1
          // Tell the runner the post was seen so it stops reposting.
          notify_post("dev.bench.ack")
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { poll() }
      }
      poll()
    }

    channel.setMethodCallHandler { call, result in
      switch call.method {
      case "beginObserved":
        if pending > 0 {
          pending -= 1
          result(true)
        } else {
          result(false)
        }
      case "postDone": notify_post("dev.bench.done"); result(nil)
      case "postStep": notify_post("dev.bench.step"); result(nil)
      case "logStep":
        // `step k n=<param> t=<unix>` → tmp/bench-steps.log; the runner
        // slices its xctrace recording by these timestamps. Identical
        // format in every contestant.
        let a = call.arguments as? [String: Any]
        let step = a?["step"] as? Int ?? -1
        let line = String(
          format: "step %d n=%d t=%.3f\n",
          step, a?["n"] as? Int ?? -1,
          Date().timeIntervalSince1970)
        let path = NSTemporaryDirectory() + "bench-steps.log"
        if !FileManager.default.fileExists(atPath: path) {
          FileManager.default.createFile(atPath: path, contents: nil)
        }
        if let h = FileHandle(forWritingAtPath: path) {
          h.seekToEndOfFile()
          h.write(Data(line.utf8))
          try? h.close()
        }
        notify_post("dev.bench.step")
        // The last ladder step: arm `done` natively NOW — after this the
        // isolate (and the main-thread channel that would carry a Dart
        // postDone) can be saturated for the whole hold. W5 has 8 steps,
        // W6 has 7; settle+hold is 5 s nominally, allow the sweeps +2 s.
        let w = UserDefaults.standard.string(forKey: "bench-workload")
        let last = w == "W5" ? 7 : (w == "W6" ? 6 : -1)
        if step == last {
          DispatchQueue.global().asyncAfter(deadline: .now() + 6) {
            notify_post("dev.bench.done")
          }
        }
        result(nil)
      case "discardBegins":
        // The runner stops reposting once it sees done, so posts queued or
        // latched at this point are ack-race backlog, not a new signal.
        pending = 0
        var f: Int32 = 0
        notify_check(beginToken, &f)
        result(nil)
      default: result(UserDefaults.standard.string(forKey: "bench-\(call.method)"))
      }
    }
  }
}
