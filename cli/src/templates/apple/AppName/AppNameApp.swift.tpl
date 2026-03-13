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
        // Register custom fonts from dependencies
        WaterUIFonts.register()

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
    private var headlessContext: WuiRootContext?
    private let isAccessory: Bool = __IS_ACCESSORY__

    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        if delegate.isAccessory {
            app.setActivationPolicy(.prohibited)
        } else {
            app.mainMenu = WaterUIMainMenu.create()
        }
        app.run()
    }



    func applicationDidFinishLaunching(_ notification: Notification) {
        // Register custom fonts from dependencies
        WaterUIFonts.register()

        if isAccessory {
            let context = WuiRootContext()
            // Force view construction so Preview::body runs and TCP server starts.
            _ = context.rootView
            headlessContext = context
            return
        }

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
