import OSLog

/// The harness logger — subsystem `dev.cherenkov.planes`, the same
/// heartbeat channel `android_planes` writes to logcat.
private let harnessLog = Logger(subsystem: "dev.cherenkov.planes", category: "harness")

/// The Rust side's `os_log` path: the variadic C `os_log` calls are not
/// linkable from arm64 slices (the SDK re-exports them for arm64e
/// only), so the heartbeat and forwarded engine messages come through
/// here. `kind`: 0 = default (the heartbeat's level, visible in
/// `log stream` without flags), 1 = error.
@_cdecl("cherenkov_planes_os_log")
func cherenkovPlanesOsLog(_ kind: UInt8, _ message: UnsafePointer<CChar>?) {
    guard let message else { return }
    let text = String(cString: message)
    if kind == 1 {
        harnessLog.error("\(text, privacy: .public)")
    } else {
        harnessLog.notice("\(text, privacy: .public)")
    }
}
