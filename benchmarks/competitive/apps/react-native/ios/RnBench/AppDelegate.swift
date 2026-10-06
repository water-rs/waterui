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

    var initialProps: [String: Any] = [
      "workload": workload
    ]
    if let step = UserDefaults.standard.string(forKey: "bench-step"),
      let n = Int(step)
    {
      initialProps["step"] = n
    }

    // Readiness = the workload's content first appearing (React Native
    // posts RCTContentDidAppearNotification once the surface has mounted
    // its first JS content) — the point every contestant posts
    // `dev.bench.ready.<contestant>.<w>` at (SwiftUI `onAppear`, UIKit
    // `viewDidAppear`, WaterUI `on_appear`). The runner waits for it
    // instead of a deep AX query on the 10k-row feed. Registered before
    // the surface starts, so the first appearance cannot be missed.
    // Every iOS contestant is installed under one shared bundle id, so
    // the post names the contestant itself; the runner waits for the id
    // of the stage entry it installed.
    readyName = "dev.bench.ready.\(AppDelegate.contestant).\(workload)"
    NotificationCenter.default.addObserver(
      self, selector: #selector(contentDidAppear(_:)),
      name: AppDelegate.contentDidAppearName, object: nil)

    factory.startReactNative(
      withModuleName: "RnBench",
      in: window,
      initialProperties: initialProps,
      launchOptions: launchOptions
    )

    return true
  }

  /// This app's contestant id in benchmarks/competitive/apple/manifest.json.
  private static let contestant = "rn"
  private static let contentDidAppearName = Notification.Name("RCTContentDidAppearNotification")
  private var readyName = ""

  /// First appearance of the workload's content: post readiness once.
  @objc private func contentDidAppear(_ note: Notification) {
    NotificationCenter.default.removeObserver(
      self, name: AppDelegate.contentDidAppearName, object: nil)
    notify_post(readyName)
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
