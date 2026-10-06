// Shared UI-test harness for the competitive benchmark — runs on the
// physical iPhone the device host drives (the only Apple benchmark
// platform). Drives one workload of one contestant per test run,
// configured through environment variables injected into the .xctestrun
// EnvironmentVariables:
//   BENCH_BUNDLE_ID  — bundle identifier of the app under test
//   BENCH_WORKLOAD   — w1 | w2 | w3 | w4 | w5 | w6 (exact lowercase; any
//                      other value fails)
//   BENCH_STEP       — capacity step for w5/w6 (one launch = one step)
//   BENCH_DRIVE      — "swipe" (coordinate drags: w2/w4/w6), "tap" (w1)
//                      or "none" (w3/w5)
//   BENCH_FLING      — JSON fling program from the manifest:
//                      {"start_fraction":0.75,"end_fraction":0.15,
//                       "duration_ms":250,"pause_s":0.35,"hold_s":0.02,
//                       "flings_down":8,"flings_up":2}
//   BENCH_DURATION   — capture seconds for every workload (required; a
//                      missing or malformed value fails). The host
//                      computes the window itself — [first owned present
//                      + warmup, + BENCH_DURATION] on its all-process
//                      trace (METHOD); the trace is readable only after
//                      the recording, so this runner starts the drive at
//                      the live dev.bench.ready post + warmup and holds
//                      the contestant for BENCH_DURATION +
//                      BENCH_ANCHOR_TOLERANCE_MS, so the window lies
//                      inside the held span.
//   BENCH_WARMUP_MS  — declared warmup between the contestant's
//                      dev.bench.ready post and the drive (required, > 0)
//   BENCH_ANCHOR_TOLERANCE_MS — how far a driven cell's drive may start
//                      from the trace's window start (required, > 0); the
//                      hold extends past the capture by the same amount
//   BENCH_RUN_NONCE  — non-zero u64 the host chose for this invocation;
//                      the recorder-go handshake matches it, so a latched
//                      signal from an earlier invocation can never
//                      release this one
//
// testWorkload does NOT launch the app until the host has armed its
// recorders. The host latches recorder-go: it has no notify channel into
// the device, so it copies `bench-recorder-go-<nonce>` into this runner's
// tmp. The file is latched, so the order in which host and runner arrive
// cannot lose the signal. Each row also records the device
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
import UIKit
import XCTest
import os

final class BenchTests: XCTestCase {
    /// The runner's marks in the host's all-process trace (the frames
    /// recorder adds the Points of Interest instrument): drive-begin is
    /// where the drive starts, measure-end where the hold ends. The same
    /// instants go to the runner log, which carries them on the wall
    /// clock — together they relate the trace clock to the host's.
    private static let marks = OSLog(subsystem: "dev.bench",
                                     category: .pointsOfInterest)
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
        guard ["swipe", "tap", "none"].contains(drive) else {
            throw HarnessError(
                description: "missing or unrecognized BENCH_DRIVE "
                    + "(got \(drive)); expected swipe|tap|none")
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
        // Thermal gate, measured by the runner itself on the device:
        // devicectl has no thermalState channel, so the check lives in
        // this process. An already-cool device proceeds immediately; one
        // still hot at the bound fails.
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
        let maxFps = UIScreen.main.maximumFramesPerSecond
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
        var metrics: [any XCTMetric] = [
            XCTMemoryMetric(application: app),
            XCTCPUMetric(application: app),
        ]
        if #available(iOS 26.0, *) {
            metrics.append(XCTHitchMetric(application: app))
        }
        // The scroll signpost only has intervals to aggregate on a scrolling
        // workload — on W1/W3 it records nothing and lands as a zeroed
        // duration/ratio pair in the report.
        if ["w2", "w4", "w6"].contains(workload) {
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
        func positive(_ key: String) throws -> Double {
            guard let raw = env[key], let v = Double(raw), v > 0 else {
                throw HarnessError(description: "missing or malformed \(key)")
            }
            return v
        }
        let warmupMs = try positive("BENCH_WARMUP_MS")
        let duration = try positive("BENCH_DURATION")
        let toleranceMs = try positive("BENCH_ANCHOR_TOLERANCE_MS")
        guard let rawNonce = env["BENCH_RUN_NONCE"],
            let nonce = UInt64(rawNonce), nonce != 0
        else {
            throw HarnessError(description: "missing or malformed BENCH_RUN_NONCE")
        }
        let hold = duration + toleranceMs / 1000.0
        dbg("testWorkload: launching \(bundleID) w=\(workload) drive=\(drive)")
        registerWorkloadReady()
        // The host arms its recorders, then latches recorder-go — launch
        // is gated on that handshake so every recorder covers the app
        // from its birth.
        try waitForRecorderGo(nonce: nonce)
        app.launch()
        dbg("launched — awaiting ready post")
        assertWorkloadReady()
        let driveAt = Date().addingTimeInterval(warmupMs / 1000.0)
        dbg("ready ok")

        // device state read on-device per row (devicectl cannot report
        // thermal/fps)
        recordDeviceState()
        // XCTest invokes a measure block iterationCount + 1 times and
        // discards the first invocation's measurements
        // (XCTMeasureOptions.iterationCount). The discarded invocation is
        // the declared warmup: it holds until readiness + warmup. The
        // recorded one is the measured span: the drive, then the hold.
        // Each invocation does its own work exactly once, so the single
        // dev.bench.begin and the host's one-shot listener belong to the
        // recorded invocation alone; any other invocation count fails.
        var invocation = 0
        measure(metrics: baseMetrics(), options: measureOptions) {
            invocation += 1
            switch invocation {
            case 1:
                let remaining = driveAt.timeIntervalSinceNow
                guard remaining >= 0 else {
                    XCTFail("XCTest started measuring \(-remaining)s after "
                        + "the declared warmup ended")
                    return
                }
                Thread.sleep(forTimeInterval: remaining)
            case 2:
                measuredSpan(hold: hold)
            default:
                XCTFail("measure block invoked \(invocation) times; the "
                    + "harness expects one discarded warmup and one "
                    + "measured invocation")
            }
        }
        guard invocation == 2 else {
            XCTFail("measure block invoked \(invocation) time(s); the drive "
                + "runs only in the second (measured) invocation")
            return
        }
    }

    /// The measured invocation: mark drive-begin, run the drive, hold
    /// until drive-begin + `hold`, mark measure-end.
    private func measuredSpan(hold: Double) {
        os_signpost(.event, log: Self.marks, name: "drive-begin")
        dbg("drive-begin")
        let end = Date().addingTimeInterval(hold)
        switch workload {
        case "w2", "w4", "w6":
            swipeFlings()
            holdWindow(until: end)
        case "w3", "w5":
            holdWindow(until: end)
        default:
            tapCounter()
            holdWindow(until: end)
        }
        os_signpost(.event, log: Self.marks, name: "measure-end")
        dbg("measure-end")
    }

    /// Holds the measured invocation open until `end`. A drive program
    /// that ran past it is a harness defect, never a longer window.
    private func holdWindow(until end: Date) {
        let remaining = end.timeIntervalSinceNow
        guard remaining >= 0 else {
            XCTFail("drive program overran the held span by "
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

    /// Blocks until the host has armed its recorders. The host has no
    /// notify channel into the device; it copies
    /// `bench-recorder-go-<nonce>` into this runner's tmp. The file is
    /// latched, so it does not matter whether the host or this runner
    /// gets here first: a DispatchSource on the directory is armed before
    /// the file is checked, so the copy is seen whenever it lands.
    private func waitForRecorderGo(nonce: UInt64) throws {
        let fired = DispatchSemaphore(value: 0)
        let queue = DispatchQueue(label: "bench.recorder-go")
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
        let ok = fired.wait(timeout: .now() + 300) == .success
        dbg("recorder-go fired=\(ok)")
        guard ok else {
            throw HarnessError(
                description: "host never armed its recorders within 300s "
                    + "(recorder-go nonce \(nonce))")
        }
    }

    // MARK: - Driving

    /// `swipe` drive: the same fling sequence for every
    /// contestant (../WORKLOADS.md) — 8 flings down then 2 back up, as
    /// coordinate drags. Not element gestures: `swipeUp` resolves the app
    /// element and waits for quiescence on every call, and a scrolling
    /// workload keeps the AX server inside the app busy long enough to
    /// stall the query past the test cap. `XCUICoordinate.press(thenDragTo:)`
    /// resolves one window-frame point and injects raw HID events without
    /// a quiescence wait — identical touches for every contestant.
    private func swipeFlings() {
        let dragAnchor: XCUIElement = app
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
