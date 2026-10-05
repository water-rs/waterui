import UIKit
import React
import React_RCTAppDelegate
import ReactAppDependencyProvider

@main
class AppDelegate: UIResponder, UIApplicationDelegate {
  var window: UIWindow?

  var reactNativeDelegate: ReactNativeDelegate?
  var reactNativeFactory: RCTReactNativeFactory?

  func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
  ) -> Bool {
    let delegate = ReactNativeDelegate()
    let factory = RCTReactNativeFactory(delegate: delegate)
    delegate.dependencyProvider = RCTAppDependencyProvider()

    reactNativeDelegate = delegate
    reactNativeFactory = factory

    window = UIWindow(frame: UIScreen.main.bounds)

    // Launch argument `-bench-workload w2` lands in NSUserDefaults'
    // NSArgumentDomain. Missing or unrecognized workload traps — a wrong
    // page must fail, never silently measure W1. Scrolling is the
    // runner's OS-level input; no drive argument exists anymore.
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

    var initialProps: [String: Any] = [
      "workload": workload
    ]
    if let step = UserDefaults.standard.string(forKey: "bench-step"),
      let n = Int(step)
    {
      initialProps["step"] = n
    }

    factory.startReactNative(
      withModuleName: "RnBench",
      in: window,
      initialProperties: initialProps,
      launchOptions: launchOptions
    )

    // The runner asserts this identifier after launch — set it once on
    // the window's root view; a missed set surfaces as a failed launch
    // check, never a blind retry.
    window?.rootViewController?.view.accessibilityIdentifier =
      "bench-workload-\(workload)"

    // BENCH_READY on stdout marks the first rendered JS frame for
    // external launch timing (device mode has no XCTest).
    NotificationCenter.default.addObserver(
      forName: Notification.Name("RCTContentDidAppearNotification"),
      object: nil,
      queue: .main
    ) { _ in
      FileHandle.standardOutput.write("BENCH_READY\n".data(using: .utf8)!)
    }

    return true
  }
}

class ReactNativeDelegate: RCTDefaultReactNativeFactoryDelegate {
  override func sourceURL(for bridge: RCTBridge) -> URL? {
    self.bundleURL()
  }

  override func bundleURL() -> URL? {
#if DEBUG
    RCTBundleURLProvider.sharedSettings().jsBundleURL(forBundleRoot: "index")
#else
    Bundle.main.url(forResource: "main", withExtension: "jsbundle")
#endif
  }
}
