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
    let rawWorkload = UserDefaults.standard.string(forKey: "bench-workload")
    guard let workload = rawWorkload,
      ["w1", "w2", "w3", "w4", "w5", "w6"].contains(workload)
    else {
      fatalError(
        "missing or unrecognized -bench-workload launch argument "
          + "(got \(rawWorkload ?? "nil")); expected w1..=w6")
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

    channel.setMethodCallHandler { call, result in
      switch call.method {
      case "postDone":
        // The app's own workload logic ends its hold by calling this;
        // there is no native-side timer.
        notify_post("dev.bench.done")
        result(nil)
      default: result(UserDefaults.standard.string(forKey: "bench-\(call.method)"))
      }
    }
  }
}
