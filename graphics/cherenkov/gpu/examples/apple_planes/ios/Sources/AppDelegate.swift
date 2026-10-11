import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        application.isIdleTimerDisabled = true
        // The brightness is owned by the Rust side: it dims only after
        // the launch arguments validate, and restores on resign-active,
        // terminate and every fatal exit path.
        return true
    }

    func applicationWillTerminate(_ application: UIApplication) {
        cherenkov_planes_brightness_restore()
    }
}
