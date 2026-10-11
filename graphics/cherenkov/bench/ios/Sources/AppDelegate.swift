import OSLog
import UIKit

private let logger = Logger(subsystem: "dev.cherenkov", category: "bench")

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    /// The status view the scene's window hosts; the runner drives it.
    let status = StatusViewController()
    private var originalBrightness: CGFloat = UIScreen.main.brightness

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        application.isIdleTimerDisabled = true
        originalBrightness = UIScreen.main.brightness
        UIScreen.main.brightness = 0
        logger.info("launch: idle timer disabled, brightness -> 0")

        let status = status
        Thread.detachNewThread { [weak self] in
            let code = BenchRunner(delegate: status).runAll()
            DispatchQueue.main.async {
                application.isIdleTimerDisabled = false
                if let brightness = self?.originalBrightness {
                    UIScreen.main.brightness = brightness
                }
                logger.info("all runs done; exit code \(code)")
                if CommandLine.arguments.contains("--exit-after-run") {
                    exit(code)
                }
            }
        }
        return true
    }
}
