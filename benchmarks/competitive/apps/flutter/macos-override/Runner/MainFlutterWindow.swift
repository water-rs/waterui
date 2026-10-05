import Cocoa
import FlutterMacOS
import notify

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    // `-bench-workload w1..=w6` via NSUserDefaults' NSArgumentDomain —
    // exact lowercase ids; missing or unrecognized traps.
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

    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    // The runner asserts this accessibility identifier after launch.
    flutterViewController.view.setAccessibilityIdentifier(
      "bench-workload-\(workload)")

    RegisterGeneratedPlugins(registry: flutterViewController)

    // Launch arguments `-bench-workload W2` land in NSUserDefaults'
    // NSArgumentDomain; expose them to Dart for workload selection.
    let channel = FlutterMethodChannel(
      name: "bench/config",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    channel.setMethodCallHandler { call, result in
      switch call.method {
      default: result(UserDefaults.standard.string(forKey: "bench-\(call.method)"))
      }
    }

    super.awakeFromNib()
  }
}
