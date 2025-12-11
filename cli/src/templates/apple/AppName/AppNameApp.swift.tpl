#if os(iOS)
import UIKit
import WaterUI

@main
class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        let window = UIWindow(frame: UIScreen.main.bounds)
        window.rootViewController = WaterUIViewController()
        window.makeKeyAndVisible()
        self.window = window
        return true
    }
}
#elseif os(macOS)
import AppKit
import WaterUI

@main
class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 800, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "__APP_NAME__"
        window.contentView = WaterUIView(frame: window.contentRect(forFrameRect: window.frame))
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
    }
}
#endif
