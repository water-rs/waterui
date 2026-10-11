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
            // The exit decision lives on this thread, not on the main
            // queue: a launch whose main queue stops being drained before
            // the completion block runs would wait forever with its result
            // already written — a blocked restore must not hold the exit.
            // exit() needs no UIKit state: the idle-timer override dies
            // with the process and the system restores screen brightness
            // when the app leaves the foreground.
            DispatchQueue.main.async {
                application.isIdleTimerDisabled = false
                screen.brightness = originalBrightness
                logger.info("restore delivered on main")
            }
            if CommandLine.arguments.contains("--exit-after-run") {
                logger.info("exiting with code \(code)")
                exit(code)
            }
        }
    }
}
