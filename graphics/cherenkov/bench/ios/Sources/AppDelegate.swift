import OSLog
import UIKit

private let logger = Logger(subsystem: "dev.cherenkov", category: "bench")

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    /// The status view the scene's window hosts; the runner drives it.
    let status = StatusViewController()
    private var started = false

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        true
    }

    func start(on screen: UIScreen) {
        guard !started else { return }
        started = true
        let application = UIApplication.shared
        let originalBrightness = screen.brightness
        application.isIdleTimerDisabled = true
        screen.brightness = 0
        logger.info("launch: idle timer disabled, brightness -> 0")

        let status = status
        Thread.detachNewThread {
            let code = BenchRunner(delegate: status).runAll()
            logger.info("bench finished with code \(code); scheduling restore")
            // The restore is delivered on the main queue and waited on
            // with a bound: brightness set through UIScreen persists until
            // the device locks, so exiting without restoring would leave
            // the screen dark. If the main queue never runs the block —
            // the failure class that used to hang the exit — the run still
            // terminates: exit() needs no UIKit state and stays on this
            // thread.
            let restored = DispatchSemaphore(value: 0)
            DispatchQueue.main.async {
                application.isIdleTimerDisabled = false
                screen.brightness = originalBrightness
                restored.signal()
                logger.info("restore delivered on main")
            }
            if restored.wait(timeout: .now() + 2) == .timedOut {
                logger.error("restore was not delivered on main; exiting anyway")
            }
            if CommandLine.arguments.contains("--exit-after-run") {
                logger.info("exiting with code \(code)")
                exit(code)
            }
        }
    }
}
