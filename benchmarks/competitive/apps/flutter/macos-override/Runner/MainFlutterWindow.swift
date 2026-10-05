import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    // `-bench-workload W1..=W6` via NSUserDefaults' NSArgumentDomain;
    // missing or unrecognized traps — never silently measure W1.
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
    var pending = 0
    var beginToken: Int32 = 0
    if notify_register_check("dev.bench.begin", &beginToken) == UInt32(NOTIFY_STATUS_OK) {
      var fired: Int32 = 0
      notify_check(beginToken, &fired)
      func poll() {
        var f: Int32 = 0
        notify_check(beginToken, &f)
        if f != 0 {
          pending += 1
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
      case "logStep":
        let a = call.arguments as? [String: Any]
        let line = String(
          format: "step %d n=%d t=%.3f\n",
          a?["step"] as? Int ?? -1, a?["n"] as? Int ?? -1,
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
        result(nil)
      case "discardBegins":
        pending = 0
        var f: Int32 = 0
        notify_check(beginToken, &f)
        result(nil)
      default: result(UserDefaults.standard.string(forKey: "bench-\(call.method)"))
      }
    }

    super.awakeFromNib()
  }
}
