import Flutter
import notify
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    // Launch arguments `-bench-workload w2` land in
    // NSUserDefaults' NSArgumentDomain. The id is an exact lowercase
    // string — a missing, uppercase or unrecognized workload traps,
    // never silently measures w1.
    _ = benchWorkload()
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    let messenger = engineBridge.applicationRegistrar.messenger()

    // Launch arguments `-bench-workload w2` land in NSUserDefaults'
    // NSArgumentDomain; expose them to Dart for workload selection.
    let config = FlutterMethodChannel(name: "bench/config", binaryMessenger: messenger)
    config.setMethodCallHandler { call, result in
      result(UserDefaults.standard.string(forKey: "bench-\(call.method)"))
    }

    // Dart reports the workload page's first frame — the readiness point
    // every contestant shares; the runner waits for
    // `dev.bench.ready.<bundle-id>.<w>` instead of a deep AX query (the
    // 10k-row feed's accessibility tree takes minutes to materialize).
    let ready = FlutterMethodChannel(name: "bench/ready", binaryMessenger: messenger)
    ready.setMethodCallHandler { call, result in
      guard call.method == "ready" else {
        result(FlutterMethodNotImplemented)
        return
      }
      let bid = Bundle.main.bundleIdentifier ?? "unknown"
      notify_post("dev.bench.ready.\(bid).\(benchWorkload())")
      result(nil)
    }
  }
}

/// `-bench-workload w1..=w6`; traps on a missing or unrecognized id.
private func benchWorkload() -> String {
  let raw = UserDefaults.standard.string(forKey: "bench-workload")
  guard let raw, ["w1", "w2", "w3", "w4", "w5", "w6"].contains(raw) else {
    fatalError(
      "missing or unrecognized -bench-workload launch argument "
        + "(got \(raw ?? "nil")); expected w1..=w6")
  }
  return raw
}
