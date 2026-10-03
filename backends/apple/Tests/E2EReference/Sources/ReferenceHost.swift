// Entry point for the SwiftUI reference host.
//
// This deliberately mirrors the app scaffold the `water` CLI generates for
// playground projects (see cli/src/templates/apple/AppName/AppNameApp.swift.tpl
// in the waterui repository): a raw AppDelegate that builds the window by hand
// so window geometry, title and content-hosting behaviour are identical on both
// sides. The only difference is that the content view is an NSHostingView /
// UIHostingController holding the SwiftUI twin instead of a WaterUIView.
//
// Launch arguments:
//   -E2EExample <name>   which twin to render (must exist in twins.txt)
//   -E2ETitle <title>    window title; passed the example's Water.toml `name`

import SwiftUI
import os

/// The twin's first-paint marker, on the same `dev.waterui` subsystem the
/// example under test reports through.
///
/// The shard used to wait a fixed two seconds after launching this host and
/// then capture. A SwiftUI cold launch in the simulator regularly needs
/// longer, and a capture taken before the first frame is a blank screen —
/// which agrees with the next blank frame, so the settle loop accepts it
/// immediately and the run reports a parity regression against a correct
/// WaterUI render. The host therefore says when it has actually drawn.
private let wuiReferenceLog = Logger(subsystem: "dev.waterui", category: "Startup")

/// Process start in nanoseconds, in the domain `wuiReferenceNowNanos()` reads.
/// Taken from the kernel's own record rather than `Date()`, both so the
/// measurement covers dyld and the static initialisers that run before this
/// code is reached and so the lazily-initialised global cannot skew it: a
/// `Date()` here stamps first access, and first access is the completion
/// block below — first paint itself — which is why the marker reported
/// `waterui_reference_first_paint_ms=0` (nightly run 35475492529). The reads
/// mirror WuiLaunchTiming in Sources/WaterUI/Core.
private let wuiReferenceLaunchNanos: UInt64 = {
  #if canImport(AppKit)
    var usage = rusage_info_v4()
    let status = withUnsafeMutablePointer(to: &usage) { pointer in
      pointer.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { rebound in
        proc_pid_rusage(getpid(), RUSAGE_INFO_V4, rebound)
      }
    }
    guard status == 0 else { return wuiReferenceNowNanos() }
    var timebase = mach_timebase_info_data_t()
    mach_timebase_info(&timebase)
    return usage.ri_proc_start_abstime * UInt64(timebase.numer) / UInt64(timebase.denom)
  #else
    // libproc is not in the iOS SDK's public module map; sysctl's
    // KERN_PROC_PID reports the same kernel start record as a wall-clock
    // timeval.
    var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
    var kp = kinfo_proc()
    var size = MemoryLayout<kinfo_proc>.stride
    guard sysctl(&mib, 4, &kp, &size, nil, 0) == 0 else { return wuiReferenceNowNanos() }
    return UInt64(kp.kp_proc.p_starttime.tv_sec) * 1_000_000_000
      + UInt64(kp.kp_proc.p_starttime.tv_usec) * 1_000
  #endif
}()

private func wuiReferenceNowNanos() -> UInt64 {
  #if canImport(AppKit)
    var timebase = mach_timebase_info_data_t()
    mach_timebase_info(&timebase)
    return mach_absolute_time() * UInt64(timebase.numer) / UInt64(timebase.denom)
  #else
    return clock_gettime_nsec_np(CLOCK_REALTIME)
  #endif
}

/// Shared "the first frame is on screen" signal for the twins. The host
/// injects it at the root and flips `painted` from the same CATransaction
/// completion that emits the first-paint marker, so observing it means the
/// window and the view hierarchy are already presenting — the earliest
/// moment a twin can safely mutate chrome that must survive the launch
/// commit (see LiquidGlassTwin's bottom accessory).
@MainActor
final class WuiReferencePresentation: ObservableObject {
  @Published var painted = false
}

/// Emits the marker once the initial render has been committed.
@MainActor
func wuiSignalReferenceFirstPaint(presentation: WuiReferencePresentation) {
  DispatchQueue.main.async {
    CATransaction.begin()
    CATransaction.setCompletionBlock {
      presentation.painted = true
      let elapsed = (wuiReferenceNowNanos() - wuiReferenceLaunchNanos) / 1_000_000
      // `notice` rather than `debug`: every `log stream` configuration the
      // shard uses captures notice, while debug needs an explicit level.
      wuiReferenceLog.notice("waterui_reference_first_paint_ms=\(elapsed, privacy: .public)")
    }
    CATransaction.commit()
  }
}

#if os(iOS)
  import UIKit

  @main
  class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(
      _: UIApplication,
      didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
      true
    }
  }

  // The window belongs to the scene, as in the scaffold: UIKit requires the
  // scene life cycle from the iOS 27 SDK on, and the Info.plist the build
  // script writes names this class by its Objective-C name.
  @objc(SceneDelegate)
  class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(
      _ scene: UIScene,
      willConnectTo _: UISceneSession,
      options _: UIScene.ConnectionOptions
    ) {
      guard let windowScene = scene as? UIWindowScene else {
        fatalError("The application scene is not a window scene: \(scene)")
      }
      let window = UIWindow(windowScene: windowScene)
      let presentation = WuiReferencePresentation()
      window.rootViewController = UIHostingController(
        rootView: TwinRoot().environmentObject(presentation))
      window.makeKeyAndVisible()
      self.window = window
      wuiSignalReferenceFirstPaint(presentation: presentation)
    }
  }
#elseif os(macOS)
  import AppKit

  @main
  class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?

    static func main() {
      let app = NSApplication.shared
      let delegate = AppDelegate()
      app.delegate = delegate
      app.run()
    }

    func applicationDidFinishLaunching(_: Notification) {
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 800, height: 600),
        styleMask: [.titled, .closable, .miniaturizable, .resizable],
        backing: .buffered,
        defer: false
      )
      if let title = UserDefaults.standard.string(forKey: "E2ETitle") {
        window.title = title
      }
      let presentation = WuiReferencePresentation()
      let hostingView = NSHostingView(rootView: TwinRoot().environmentObject(presentation))
      // Do not let the SwiftUI content's ideal size drive the window size —
      // the waterui scaffold pins the content rect at 800×600 unconditionally.
      hostingView.sizingOptions = []
      window.contentView = hostingView
      window.center()
      window.makeKeyAndOrderFront(nil)
      self.window = window
      wuiSignalReferenceFirstPaint(presentation: presentation)

      // Mirror the chrome flags the WaterUI backend applies once a window has
      // a toolbar (WuiWindowToolbar): unified style + full-size content, which
      // is what lets a NavigationSplitView sidebar run the window's full
      // height with the traffic lights inside it — the presentation Apple's
      // own apps use. SwiftUI installs window.toolbar on first layout, so the
      // check runs after a turn of the main loop.
      DispatchQueue.main.async {
        if window.toolbar != nil {
          window.toolbarStyle = .unified
          window.styleMask.insert(.fullSizeContentView)
        }
      }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool {
      true
    }
  }
#endif
