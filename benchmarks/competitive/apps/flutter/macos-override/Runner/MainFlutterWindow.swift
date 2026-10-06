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

    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)

    // Launch arguments `-bench-workload w2` land in NSUserDefaults'
    // NSArgumentDomain; expose them to Dart for workload selection.
    let messenger = flutterViewController.engine.binaryMessenger
    let config = FlutterMethodChannel(name: "bench/config", binaryMessenger: messenger)
    config.setMethodCallHandler { call, result in
      result(UserDefaults.standard.string(forKey: "bench-\(call.method)"))
    }

    // Dart reports the workload page's first frame — the readiness point
    // every contestant shares; the runner waits for
    // `dev.bench.ready.<bundle-id>.<w>` instead of a deep AX query.
    let ready = FlutterMethodChannel(name: "bench/ready", binaryMessenger: messenger)
    ready.setMethodCallHandler { call, result in
      guard call.method == "ready" else {
        result(FlutterMethodNotImplemented)
        return
      }
      let bid = Bundle.main.bundleIdentifier ?? "unknown"
      notify_post("dev.bench.ready.\(bid).\(workload)")
      result(nil)
    }

    super.awakeFromNib()
  }
}
