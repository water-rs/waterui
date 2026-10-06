// Shared UI-test harness for the competitive benchmark.
// Drives one workload of one contestant per test run, configured through
// environment variables injected into the .xctestrun EnvironmentVariables:
//   BENCH_BUNDLE_ID  — bundle identifier of the app under test
//   BENCH_WORKLOAD   — w1 | w2 | w3 | w4 | w5 | w6 (exact lowercase; any
//                      other value fails)
//   BENCH_STEP       — capacity step for w5/w6 (one launch = one step)
//   BENCH_DRIVE      — "swipe" (coordinate drags, iOS device), "wheel"
//                      (host-side CGEvent scroll-wheel detents — iOS
//                      Simulator and macOS; the host driver starts on the
//                      single `dev.bench.begin` this runner posts when the
//                      measure block opens and ends the cell with its own
//                      `dev.bench.end`), "tap" (w1) or "none"
//   BENCH_FLING      — JSON fling program from the manifest:
//                      {"start_fraction":0.75,"end_fraction":0.15,
//                       "duration_ms":250,"pause_s":0.35,"hold_s":0.02,
//                       "flings_down":8,"flings_up":2}
//   BENCH_DURATION   — capture seconds for every workload (required; a
//                      missing or malformed value fails). The measure
//                      block is exactly this long: the drive program runs
//                      inside it and the remainder is held, so every cell
//                      captures the same declared window (METHOD).
//   BENCH_WARMUP_MS  — declared warmup between the contestant's
//                      dev.bench.ready first-frame post and the start
//                      of the measure window (required, > 0 — METHOD:
//                      window = first owned present + warmup)
//   BENCH_RUN_NONCE  — non-zero u64 the host chose for this invocation;
//                      the recorder-go handshake matches it, so a latched
//                      signal from an earlier invocation can never
//                      release this one
//   BENCH_NO_HITCH   — "1" drops XCTHitchMetric (used when the platform
//                      cannot record it; see bench.py)
//
// testWorkload does NOT launch the app until the host has armed its
// recorders. The host latches recorder-go: on macOS and the iOS
// Simulator it holds a registration on `dev.bench.recorder` whose notify
// state is BENCH_RUN_NONCE, then posts it; on ios-device (no notify
// channel into the device) it copies `bench-recorder-go-<nonce>` into
// this runner's tmp. Both are latched, so the order in which host and
// runner arrive cannot lose the signal. Each row also records the device
// state read INSIDE this process (ProcessInfo.thermalState, screen max
// fps — devicectl cannot report them) into the runner log.
//
// Launch arguments follow one convention on every contestant:
//   -bench-workload <w1|w2|w3|w4|w5|w6> [-bench-step N]
// which lands in NSUserDefaults' NSArgumentDomain (and argv); N is the
// ladder value itself (200…25600 for w5, 1…64 for w6). Apps trap when
// the workload is missing or unrecognized and post
// `dev.bench.ready.<bundle-id>.<W>` over Darwin notify when the workload
// view first appears — a wrong page fails the test instead of measuring
// the Hello page. The assertion goes over notify (not an AX query)
// because materializing the accessibility tree of the 10k-row feed
// blocks a descendants query for minutes.
//
// The same metrics are attached for every contestant — XCTest measures the
// app from outside (Core Animation commits, memory footprint, launch
// interval), so nothing is measured by framework code.

import Darwin
import XCTest
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

final class BenchTests: XCTestCase {
    private var app: XCUIApplication!
    private var dbgURL: URL = {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("bench-runner.log")
    }()
    private func dbg(_ s: String) {
        let line = "\(Date().timeIntervalSince1970) \(s)\n"
        if let h = try? FileHandle(forWritingTo: dbgURL) {
            h.seekToEndOfFile(); h.write(line.data(using: .utf8)!)
            try? h.close()
        } else {
            try? line.write(to: dbgURL, atomically: true, encoding: .utf8)
        }
    }
    private var bundleID: String = ""
    private var workload: String = ""
    private var drive: String = ""

    /// A harness precondition that does not hold — thrown from setUp so
    /// the test body never runs against a half-configured runner.
    struct HarnessError: Error, CustomStringConvertible {
        let description: String
    }

    override func setUpWithError() throws {
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        bundleID = env["BENCH_BUNDLE_ID"] ?? ""
        guard !bundleID.isEmpty else {
            throw HarnessError(description: "BENCH_BUNDLE_ID not set")
        }
        workload = env["BENCH_WORKLOAD"] ?? ""
        drive = env["BENCH_DRIVE"] ?? ""
        guard ["w1", "w2", "w3", "w4", "w5", "w6"].contains(workload) else {
            throw HarnessError(
                description: "missing or unrecognized BENCH_WORKLOAD "
                    + "(got \(workload)); expected w1..w6")
        }
        guard ["swipe", "wheel", "tap", "none"].contains(drive) else {
            throw HarnessError(
                description: "missing or unrecognized BENCH_DRIVE "
                    + "(got \(drive)); expected swipe|wheel|tap|none")
        }
        app = XCUIApplication(bundleIdentifier: bundleID)
        app.launchArguments = ["-bench-workload", workload]
        if ["w5", "w6"].contains(workload) {
            let step = env["BENCH_STEP"] ?? ""
            guard let n = Int(step), n > 0 else {
                throw HarnessError(
                    description: "capacity workload \(workload) requires "
                        + "BENCH_STEP (a ladder value; got \(step))")
            }
            app.launchArguments += ["-bench-step", String(n)]
        }
        // Thermal gate, measured by the runner itself on every Apple
        // platform (macOS included — ProcessInfo reports the host's
        // thermal pressure there): devicectl has no thermalState
        // channel, so the check lives in this process. An already-cool
        // host proceeds immediately; one still hot at the bound fails.
        try waitForNominalThermal(budget: 600)
    }

    /// Blocks until ProcessInfo reports a nominal thermal state. The
    /// wake-up is thermalStateDidChangeNotification itself: the
    /// expectation observes it from before the state is read (a
    /// transition between the read and the wait cannot be lost), and
    /// XCTWaiter runs the run loop the notification is delivered on.
    private func waitForNominalThermal(budget: TimeInterval) throws {
        let cooled = XCTNSNotificationExpectation(
            name: ProcessInfo.thermalStateDidChangeNotification)
        cooled.handler = { _ in
            ProcessInfo.processInfo.thermalState == .nominal
        }
        if ProcessInfo.processInfo.thermalState == .nominal { return }
        dbg("thermal wait: state="
            + "\(ProcessInfo.processInfo.thermalState.rawValue)")
        guard XCTWaiter().wait(for: [cooled], timeout: budget) == .completed
        else {
            throw HarnessError(
                description: "thermal cool-down exceeded \(budget)s: state="
                    + "\(ProcessInfo.processInfo.thermalState.rawValue)")
        }
    }

    /// thermalState/maximumFramesPerSecond as observed inside the runner
    /// process — devicectl cannot report either.
    private func recordDeviceState() {
        var maxFps = -1
        #if os(iOS)
        maxFps = UIScreen.main.maximumFramesPerSecond
        #elseif os(macOS)
        maxFps = NSScreen.main?.maximumFramesPerSecond ?? -1
        #endif
        dbg("device-record "
            + "thermal=\(ProcessInfo.processInfo.thermalState.rawValue) "
            + "maxFps=\(maxFps)")
    }

    override func tearDownWithError() throws {
        app?.terminate()
        app = nil
    }

    // MARK: - Metrics

    private func baseMetrics() -> [any XCTMetric] {
        let env = ProcessInfo.processInfo.environment
        var metrics: [any XCTMetric] = [
            XCTMemoryMetric(application: app),
            XCTCPUMetric(application: app),
        ]
        if env["BENCH_NO_HITCH"] != "1",
            #available(macOS 26.0, iOS 26.0, *)
        {
            metrics.append(XCTHitchMetric(application: app))
        }
        // The scroll signpost only has intervals to aggregate on a scrolling
        // workload — on W1/W3 it records nothing and lands as a zeroed
        // duration/ratio pair in the report.
        if ["w2", "w4", "w6"].contains(workload),
            #available(macOS 11.0, iOS 15.0, *)
        {
            metrics.append(XCTOSSignpostMetric.scrollingAndDecelerationMetric)
        }
        return metrics
    }

    private var measureOptions: XCTMeasureOptions {
        let o = XCTMeasureOptions()
        o.iterationCount = 1
        return o
    }

    private func anyElement(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier].firstMatch
    }

    private var readyFd: Int32 = -1
    private var readyToken: Int32 = 0
    private var readyRegistered = false

    /// Registers `dev.bench.ready.<bundle-id>.<W>` BEFORE app.launch as a
    /// notify file descriptor: only posts after registration are
    /// delivered, so a stale flag from an earlier cell's identical post
    /// can never satisfy the wait — an app whose delegate never runs
    /// reads "not ready", not a latched flag. The fd is also the wait
    /// itself: assertWorkloadReady blocks on poll(2), never on
    /// notify_check + sleep polling.
    private func registerWorkloadReady() {
        let name = "dev.bench.ready.\(bundleID).\(workload)"
        guard notify_register_file_descriptor(name, &readyFd, 0,
                                              &readyToken)
                == UInt32(NOTIFY_STATUS_OK)
        else {
            XCTFail("notify_register_file_descriptor(\(name)) failed")
            return
        }
        readyRegistered = true
    }

    /// The app posts `dev.bench.ready.<bundle-id>.<W>` over Darwin notify
    /// once it has resolved its -bench-workload argument; absence of the
    /// post means the harness args never reached it and the run is invalid
    /// rather than a W1 measurement. The wait blocks on the fd itself.
    private func assertWorkloadReady() {
        let name = "dev.bench.ready.\(bundleID).\(workload)"
        guard readyRegistered else { XCTFail("ready token unregistered"); return }
        defer {
            notify_cancel(readyToken)
            close(readyFd)
            readyRegistered = false
        }
        var fired = false
        var pfd = pollfd(fd: readyFd, events: Int16(POLLIN), revents: 0)
        if poll(&pfd, 1, 60_000) > 0,
           (pfd.revents & Int16(POLLIN)) != 0 {
            var buf: UInt64 = 0
            _ = read(readyFd, &buf, MemoryLayout<UInt64>.size)
            fired = true
        }
        XCTAssertTrue(
            fired,
            "app never posted '\(name)' — wrong or missing "
                + "-bench-workload handling")
    }

    // MARK: - Tests

    /// Cold launch to first frame.
    func testLaunch() throws {
        measure(metrics: [XCTApplicationLaunchMetric()], options: measureOptions) {
            app.launch()
            app.terminate()
        }
    }

    /// Steady-state and peak memory + hitch metrics while the workload runs.
    func testWorkload() throws {
        let env = ProcessInfo.processInfo.environment
        guard let rawWarmup = env["BENCH_WARMUP_MS"],
            let warmupMs = Double(rawWarmup), warmupMs > 0
        else {
            throw HarnessError(description: "missing or malformed BENCH_WARMUP_MS")
        }
        guard let rawDuration = env["BENCH_DURATION"],
            let duration = Double(rawDuration), duration > 0
        else {
            throw HarnessError(description: "missing or malformed BENCH_DURATION")
        }
        guard let rawNonce = env["BENCH_RUN_NONCE"],
            let nonce = UInt64(rawNonce), nonce != 0
        else {
            throw HarnessError(description: "missing or malformed BENCH_RUN_NONCE")
        }
        dbg("testWorkload: launching \(bundleID) w=\(workload) drive=\(drive)")
        registerWorkloadReady()
        // The host arms its recorders, then latches recorder-go — launch
        // is gated on that handshake so every recorder covers the app
        // from its birth.
        try waitForRecorderGo(nonce: nonce)
        app.launch()
        dbg("launched — awaiting ready post")
        assertWorkloadReady()
        dbg("ready ok")

        // METHOD: the measure window opens the declared warmup after
        // the contestant's first-frame post — the drive program runs
        // entirely inside the window, identically for every contestant
        Thread.sleep(forTimeInterval: warmupMs / 1000.0)
        dbg("warmup done")

        // device state read on-device per row (devicectl cannot report
        // thermal/fps); the measure-window markers let the host slice
        // its samplers and traces to exactly the measure block
        recordDeviceState()
        dbg("measure-begin")
        measure(metrics: baseMetrics(), options: measureOptions) {
            // One window definition: every cell captures exactly
            // BENCH_DURATION seconds. The drive runs inside it; the
            // remainder is held. The host wheel driver owns its cell's
            // end itself, at begin + the same duration.
            let windowEnd = Date().addingTimeInterval(duration)
            switch workload {
            case "w2", "w4", "w6":
                if drive == "wheel" {
                    awaitHostWheelDrive(duration: duration)
                    return
                }
                swipeFlings()
            case "w3", "w5":
                break
            default:
                tapCounter()
            }
            holdWindow(until: windowEnd)
        }
        dbg("measure-end")
    }

    /// Holds the measure block open until the declared capture window
    /// ends. A drive program that ran past the window is a harness
    /// defect, never a longer window.
    private func holdWindow(until end: Date) {
        let remaining = end.timeIntervalSinceNow
        guard remaining >= 0 else {
            XCTFail("drive program overran the capture window by "
                + "\(-remaining)s")
            return
        }
        Thread.sleep(forTimeInterval: remaining)
    }

    /// W1: tap the counter a fixed number of times. The button is
    /// asserted, not probed — a contestant missing its workload content
    /// fails the row instead of silently tapping nothing.
    private func tapCounter() {
        var button = anyElement("increment-button")
        if !button.exists {
            button = app.buttons["Increment"].firstMatch
        }
        XCTAssertTrue(button.waitForExistence(timeout: 10),
                      "no increment button — workload content not found")
        for _ in 0..<20 {
            button.tap()
            Thread.sleep(forTimeInterval: 0.05)
        }
    }

    /// Blocks until the host has armed its recorders. Both channels are
    /// latched by the host, so it does not matter whether the host or
    /// this runner gets here first:
    /// - macOS / iOS Simulator: the host holds a registration on
    ///   `dev.bench.recorder` whose notify state is `nonce`, then posts
    ///   it. The dispatch registration is armed before the state is
    ///   read, so a post after the read wakes it and a post before it
    ///   is visible in the state.
    /// - ios-device: the host has no notify channel into the device; it
    ///   copies `bench-recorder-go-<nonce>` into this runner's tmp. A
    ///   DispatchSource on the directory is armed before the file is
    ///   checked, so the copy is seen whenever it lands.
    private func waitForRecorderGo(nonce: UInt64) throws {
        let fired = DispatchSemaphore(value: 0)
        let queue = DispatchQueue(label: "bench.recorder-go")
        #if os(iOS) && !targetEnvironment(simulator)
        let dir = FileManager.default.temporaryDirectory
        let sentinel = dir.appendingPathComponent("bench-recorder-go-\(nonce)").path
        let dirFd = open(dir.path, O_EVTONLY)
        guard dirFd >= 0 else {
            throw HarnessError(description: "cannot watch \(dir.path): errno \(errno)")
        }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: dirFd, eventMask: .write, queue: queue)
        source.setEventHandler {
            if FileManager.default.fileExists(atPath: sentinel) { fired.signal() }
        }
        source.setCancelHandler { close(dirFd) }
        source.resume()
        defer { source.cancel() }
        queue.sync {
            if FileManager.default.fileExists(atPath: sentinel) { fired.signal() }
        }
        #else
        var token: Int32 = 0
        let matches: (Int32) -> Bool = { t in
            var state: UInt64 = 0
            return notify_get_state(t, &state) == UInt32(NOTIFY_STATUS_OK)
                && state == nonce
        }
        guard notify_register_dispatch("dev.bench.recorder", &token, queue, { t in
            if matches(t) { fired.signal() }
        }) == UInt32(NOTIFY_STATUS_OK) else {
            throw HarnessError(
                description: "notify_register_dispatch(dev.bench.recorder) failed")
        }
        defer { notify_cancel(token) }
        queue.sync {
            if matches(token) { fired.signal() }
        }
        #endif
        let ok = fired.wait(timeout: .now() + 300) == .success
        dbg("recorder-go fired=\(ok)")
        guard ok else {
            throw HarnessError(
                description: "host never armed its recorders within 300s "
                    + "(recorder-go nonce \(nonce))")
        }
    }

    // MARK: - Driving

    /// `wheel` drive (iOS Simulator and macOS): the host's own process
    /// posts CGEvent scroll-wheel detents — the identical fling protocol —
    /// into the contestant. This runner posts `dev.bench.begin` exactly
    /// once, when the measure block opens; the host armed its listener
    /// before it released recorder-go, so the post cannot be missed. The
    /// host driver ends the cell with `dev.bench.end` at begin + the
    /// declared duration — apps never post it.
    private func awaitHostWheelDrive(duration: Double) {
        var fd: Int32 = -1
        var endToken: Int32 = 0
        guard notify_register_file_descriptor("dev.bench.end", &fd, 0,
                    &endToken) == UInt32(NOTIFY_STATUS_OK)
        else {
            XCTFail("wheel drive: notify_register_file_descriptor failed")
            return
        }
        defer { notify_cancel(endToken); close(fd) }
        // registered before the begin post: the driver's end can only
        // follow its begin, so it always lands on this descriptor
        notify_post("dev.bench.begin")
        var pfd = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
        // the driver ends the cell at begin + duration; the bound only
        // catches a driver that died
        let boundMs = Int32((duration + 30) * 1000)
        let fired = poll(&pfd, 1, boundMs) > 0
            && (pfd.revents & Int16(POLLIN)) != 0
        if fired {
            var buf: Int32 = 0
            _ = read(fd, &buf, MemoryLayout<Int32>.size)
        }
        XCTAssertTrue(
            fired,
            "wheel drive: host driver never posted 'dev.bench.end' within "
                + "\(duration + 30)s of begin")
    }

    /// `swipe` drive (iOS device): the same fling sequence for every
    /// contestant (../WORKLOADS.md) — 8 flings down then 2 back up, as
    /// coordinate drags. Not element gestures: `swipeUp` resolves the app
    /// element and waits for quiescence on every call, and a scrolling
    /// workload keeps the AX server inside the app busy long enough to
    /// stall the query past the test cap. `XCUICoordinate.press(thenDragTo:)`
    /// resolves one window-frame point and injects raw HID events without
    /// a quiescence wait — identical touches for every contestant.
    private func swipeFlings() {
        // On macOS the XCUIApplication element's frame is empty, so a
        // normalized coordinate on it resolves to an infinite point;
        // anchor the drags to the app's first window instead.
        #if os(macOS)
        let dragAnchor: XCUIElement = app.windows.firstMatch
        #else
        let dragAnchor: XCUIElement = app
        #endif
        // Fling program comes from the manifest via BENCH_FLING — a
        // missing or malformed program fails the row, never defaults.
        struct Fling: Decodable {
            var start_fraction: Double
            var end_fraction: Double
            var duration_ms: Double
            var pause_s: Double
            var hold_s: Double
            var flings_down: Int
            var flings_up: Int
        }
        guard let raw = ProcessInfo.processInfo.environment["BENCH_FLING"],
            let data = raw.data(using: .utf8),
            let fling = try? JSONDecoder().decode(Fling.self, from: data)
        else {
            XCTFail("missing or malformed BENCH_FLING program")
            return
        }
        // spec endpoints: 75% -> 15% of the scroll surface's height over
        // 250 ms — declared absolutely in the manifest
        let dragStart = dragAnchor.coordinate(
            withNormalizedOffset: CGVector(dx: 0.5, dy: fling.start_fraction))
        let dragEnd = dragAnchor.coordinate(
            withNormalizedOffset: CGVector(dx: 0.5, dy: fling.end_fraction))
        let anchorH = dragAnchor.frame.height
        guard anchorH > 0, fling.duration_ms > 0 else {
            XCTFail("fling geometry degenerate: anchor height \(anchorH), "
                + "duration \(fling.duration_ms)ms")
            return
        }
        // withVelocity: takes points/second — the declared endpoints'
        // distance over the declared duration
        let velocity = XCUIGestureVelocity(
            CGFloat(abs(fling.start_fraction - fling.end_fraction)) * anchorH
                / CGFloat(fling.duration_ms / 1000))
        for _ in 0..<fling.flings_down {
            dragStart.press(forDuration: fling.hold_s, thenDragTo: dragEnd,
                            withVelocity: velocity, thenHoldForDuration: 0)
            Thread.sleep(forTimeInterval: fling.pause_s)
        }
        for _ in 0..<fling.flings_up {
            dragEnd.press(forDuration: fling.hold_s, thenDragTo: dragStart,
                          withVelocity: velocity, thenHoldForDuration: 0)
            Thread.sleep(forTimeInterval: fling.pause_s)
        }
    }
}
