// Shared UI-test harness for the competitive benchmark.
// Drives one workload of one contestant per test run, configured through
// environment variables injected into the .xctestrun EnvironmentVariables:
//   BENCH_BUNDLE_ID  — bundle identifier of the app under test
//   BENCH_WORKLOAD   — w1 | w2 | w3 | w4 | w5 | w6 (exact lowercase; any
//                      other value fails)
//   BENCH_STEP       — capacity step for w5/w6 (one launch = one step)
//   BENCH_DRIVE      — "swipe" (coordinate drags on the window, iOS
//                      device/macOS) or "wheel" (host-side CGEvent scroll
//                      into the window — ios-sim/AppKit; the runner's own
//                      process drives it, started by the `dev.bench.begin`
//                      post inside the measure block and ended by its
//                      `dev.bench.end` — the driver's own name)
//   BENCH_FLING      — JSON fling program from the manifest:
//                      {"distance_fraction":0.6,"pause_s":0.35,
//                       "hold_s":0.02,"flings_down":8,"flings_up":2}
//   BENCH_DURATION   — measurement seconds for w3/w5 (required; a
//                      missing or malformed value fails)
//   BENCH_WARMUP_MS  — declared warmup between the contestant's
//                      dev.bench.ready first-frame post and the start
//                      of the measure window (required, > 0 — METHOD:
//                      window = first owned present + warmup)
//   BENCH_NO_HITCH   — "1" drops XCTHitchMetric (used when the platform
//                      cannot record it; see bench.py)
//
// testWorkload does NOT launch the app until the host posts
// `dev.bench.recorder` (or drops a bench-recorder-go sentinel into this
// runner's tmp on ios-device, where the host cannot reach the device's
// notify namespace) — the runner's xctrace recorders arm before launch
// so they bind at process birth. Each row also records the device state
// read INSIDE this process (ProcessInfo.thermalState, screen max fps —
// devicectl cannot report them) into the runner log.
//
// Launch arguments follow one convention on every contestant:
//   -bench-workload <w1|w2|w3|w4|w5|w6> [-bench-step N]
// which lands in NSUserDefaults' NSArgumentDomain (and argv). Apps trap
// when the workload is missing or unrecognized, and expose their selected
// workload as the accessibility identifier `bench-workload-<id>`. The runner
// asserts it through the `dev.bench.ready.<bundle-id>.<W>` Darwin
// notification each app posts once its argument is resolved — a wrong page
// fails the test instead of measuring the Hello page. The assertion goes
// over notify (not an AX query) because materializing the accessibility
// tree of the 10k-row feed blocks a descendants query for minutes.
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

    override func setUpWithError() throws {
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        bundleID = env["BENCH_BUNDLE_ID"] ?? ""
        guard !bundleID.isEmpty else {
            XCTFail("BENCH_BUNDLE_ID not set")
            return
        }
        workload = env["BENCH_WORKLOAD"] ?? ""
        drive = env["BENCH_DRIVE"] ?? ""
        guard ["w1", "w2", "w3", "w4", "w5", "w6"].contains(workload) else {
            XCTFail("missing or unrecognized BENCH_WORKLOAD "
                + "(got \(workload)); expected w1..w6")
            return
        }
        guard ["swipe", "wheel", "tap", "none"].contains(drive) else {
            XCTFail("missing or unrecognized BENCH_DRIVE "
                + "(got \(drive)); expected swipe|wheel|tap|none")
            return
        }
        app = XCUIApplication(bundleIdentifier: bundleID)
        app.launchArguments = ["-bench-workload", workload]
        if ["w5", "w6"].contains(workload) {
            let step = env["BENCH_STEP"] ?? ""
            guard Int(step) != nil else {
                XCTFail("capacity workload \(workload) requires BENCH_STEP")
                return
            }
            app.launchArguments += ["-bench-step", step]
        }
        // Thermal gate, measured by the runner itself: devicectl has no
        // thermalState channel, so the check lives on-device. Cool-down
        // is a bounded wait on the thermal-state notification, not a
        // fixed sleep — an already-cool device proceeds immediately and
        // a device still hot at the bound fails the row.
        try waitForNominalThermal(budget: 600)
    }

    /// Waits until ProcessInfo reports a nominal thermal state, woken by
    /// thermalStateDidChangeNotification; throws past the budget.
    private func waitForNominalThermal(budget: TimeInterval) throws {
        let deadline = Date().addingTimeInterval(budget)
        while ProcessInfo.processInfo.thermalState != .nominal {
            if Date() > deadline {
                XCTFail("thermal cool-down exceeded \(budget)s: "
                    + "state=\(ProcessInfo.processInfo.thermalState.rawValue)")
                return
            }
            let sem = DispatchSemaphore(value: 0)
            let obs = NotificationCenter.default.addObserver(
                forName: ProcessInfo.thermalStateDidChangeNotification,
                object: nil, queue: nil) { _ in sem.signal() }
            // the notification is the wake-up; the semaphore's timeout
            // carries the deadline discipline — no shared var across
            // queues
            let wake = Date().addingTimeInterval(min(30, budget))
            while ProcessInfo.processInfo.thermalState != .nominal
                && Date() < wake && Date() < deadline {
                _ = sem.wait(timeout: .now() + 0.2)
            }
            NotificationCenter.default.removeObserver(obs)
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
        dbg("testWorkload: launching \(bundleID) w=\(workload) drive=\(drive)")
        registerWorkloadReady()
        // The host arms its recorders, then posts dev.bench.recorder —
        // launch is gated on that handshake so an xctrace attach binds
        // at the app's birth, not whenever a fixed sleep happened to end.
        waitForRecorderGo()
        app.launch()
        dbg("launched — awaiting ready post")
        assertWorkloadReady()
        dbg("ready ok")

        guard let rawWarmup = ProcessInfo.processInfo
            .environment["BENCH_WARMUP_MS"],
            let warmupMs = Double(rawWarmup), warmupMs > 0
        else {
            XCTFail("missing or malformed BENCH_WARMUP_MS")
            return
        }
        // METHOD: the measure window opens the declared warmup after
        // the contestant's first-frame post — the drive program runs
        // entirely inside the window, identically for every contestant
        Thread.sleep(forTimeInterval: warmupMs / 1000.0)
        dbg("warmup done")

        guard let rawDuration = ProcessInfo.processInfo
            .environment["BENCH_DURATION"],
            let duration = Double(rawDuration), duration > 0
        else {
            XCTFail("missing or malformed BENCH_DURATION")
            return
        }

        // device state read on-device per row (devicectl cannot report
        // thermal/fps); the measure-window markers let the host slice
        // its CPU sampler to exactly the measure block
        recordDeviceState()
        dbg("measure-begin")
        measure(metrics: baseMetrics(), options: measureOptions) {
            switch workload {
            case "w2", "w4", "w6":
                driveScroll(duration: duration)
            case "w3", "w5":
                Thread.sleep(forTimeInterval: duration)
            default:
                // W1: tap the counter a fixed number of times. The
                // button is asserted, not probed — a contestant missing
                // its workload content fails the row instead of
                // silently tapping nothing.
                var button = anyElement("increment-button")
                if !button.exists {
                    button = app.buttons["Increment"].firstMatch
                }
                XCTAssertTrue(button.waitForExistence(timeout: 10),
                              "no increment button — workload content "
                              + "not found")
                for _ in 0..<20 {
                    button.tap()
                    Thread.sleep(forTimeInterval: 0.05)
                }
            }
        }
        dbg("measure-end")
    }

    /// Blocks until the host has armed its recorders: `dev.bench.recorder`
    /// on Darwin notify, or the bench-recorder-go sentinel in this
    /// runner's tmp on ios-device (the host has no notify channel into a
    /// physical device — devicectl copy is the side channel).
    private func waitForRecorderGo() {
        // Event sources, not polling: `dev.bench.recorder` on a notify
        // file descriptor blocks in poll(2). On ios-device the host has
        // NO notify channel into the device, so the bench-recorder-go
        // sentinel (devicectl copy) is the only signalling path — the
        // file check rides each poll wake for that path only.
        var fd: Int32 = -1
        var tok: Int32 = 0
        guard notify_register_file_descriptor("dev.bench.recorder",
                    &fd, 0, &tok) == UInt32(NOTIFY_STATUS_OK)
        else {
            XCTFail("notify_register_file_descriptor(dev.bench.recorder) failed")
            return
        }
        defer { notify_cancel(tok); close(fd) }
        let sentinel = FileManager.default.temporaryDirectory
            .appendingPathComponent("bench-recorder-go")
        try? FileManager.default.removeItem(at: sentinel)
        let deadline = Date().addingTimeInterval(300)
        var fired = false
        while !fired && Date() < deadline {
            var pfd = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
            // the wake interval only bounds the sentinel-check cadence —
            // the notify fd still delivers its post the moment it lands
            let rc = poll(&pfd, 1, Int32(max(0, min(2000,
                deadline.timeIntervalSinceNow * 1000))))
            if rc > 0, (pfd.revents & Int16(POLLIN)) != 0 {
                var buf: UInt64 = 0
                _ = read(fd, &buf, MemoryLayout<UInt64>.size)
                fired = true
            }
            if !fired {
                fired = FileManager.default
                    .fileExists(atPath: sentinel.path)
            }
        }
        dbg("recorder-go fired=\(fired)")
        XCTAssertTrue(fired,
                      "host never armed recorders (dev.bench.recorder)")
    }

    // MARK: - Driving

    /// Same fling sequence for every contestant (../WORKLOADS.md): 8
    /// flings down then 2 back up. `swipe` = coordinate drags; `wheel` =
    /// the host's own process posts CGEvent scroll-wheel detents into the
    /// window (iOS Simulator and AppKit, where gesture synthesis either
    /// stalls on a timed-out AX query or has no swipeable hit target) —
    /// the test only opens the measure window with `dev.bench.begin` and
    /// waits for the host driver's `dev.bench.end` — the driver owns
    /// cell end; apps never post it. w6 uses the same
    /// drive kind as w2/w4 — one launch renders one pinned step.
    private func driveScroll(duration: Double) {
        if drive == "wheel" {
            // The host driver's program is the identical fling protocol;
            // `begin` opens its window, `end` closes the wait. The wait
            // blocks on the notification's file descriptor — posts are
            // reposted on each poll wake until an end is seen (the
            // driver may arm after the first post).
            var fd: Int32 = -1
            var doneToken: Int32 = 0
            guard notify_register_file_descriptor("dev.bench.end", &fd, 0,
                        &doneToken) == UInt32(NOTIFY_STATUS_OK)
            else {
                XCTFail("wheel drive: notify_register_file_descriptor failed")
                return
            }
            defer { notify_cancel(doneToken); close(fd) }
            let deadline = Date().addingTimeInterval(max(600, duration * 2))
            var fired = false
            var lastPost = Date.distantPast
            while !fired && Date() < deadline {
                if Date().timeIntervalSince(lastPost) > 5 {
                    notify_post("dev.bench.begin")
                    lastPost = Date()
                }
                var pfd = pollfd(fd: fd, events: Int16(POLLIN),
                                 revents: 0)
                let rc = poll(&pfd, 1, Int32(max(0, min(1000,
                    deadline.timeIntervalSinceNow * 1000))))
                if rc > 0, (pfd.revents & Int16(POLLIN)) != 0 {
                    var buf: UInt64 = 0
                    _ = read(fd, &buf, MemoryLayout<UInt64>.size)
                    fired = true
                }
            }
            XCTAssertTrue(
                fired,
                "wheel drive: host driver never posted 'dev.bench.end'")
            return
        }
        // Coordinate drags, not element gestures: `swipeUp` resolves the app
        // element and waits for quiescence on every call, and a scrolling
        // workload keeps the AX server inside the app busy long enough to stall
        // the query past the test cap. `XCUICoordinate.press(thenDragTo:)`
        // resolves one window-frame point and injects raw HID events without a
        // quiescence wait — identical touches for every contestant.
        // On macOS the XCUIApplication element's frame is empty, so a
        // normalized coordinate on it resolves to an infinite point; anchor
        // the drags to the app's first window instead.
        #if os(macOS)
        let dragAnchor: XCUIElement = app.windows.firstMatch
        dbg("probe: app.exists=\(app.exists) windows=\(app.windows.count) "
            + "anchor.exists=\(dragAnchor.exists) frame=\(dragAnchor.frame)")
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
        // 250 ms — declared absolutely in the manifest, never centred
        // from a distance fraction
        let dragStart = dragAnchor.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: fling.start_fraction))
        let dragEnd = dragAnchor.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: fling.end_fraction))
        let anchorH = dragAnchor.frame.height
        guard anchorH > 0, fling.duration_ms > 0 else {
            XCTFail("fling geometry degenerate: anchor height \(anchorH), duration \(fling.duration_ms)ms")
            return
        }
        // withVelocity: takes points/second — the declared endpoints'
        // pixel distance over the declared duration
        let velocity = CGFloat(abs(fling.start_fraction - fling.end_fraction)) * anchorH / CGFloat(fling.duration_ms / 1000)
        for _ in 0..<fling.flings_down {
            dragStart.press(forDuration: fling.hold_s, thenDragTo: dragEnd, withVelocity: velocity, thenHoldForDuration: 0)
            Thread.sleep(forTimeInterval: fling.pause_s)
        }
        for _ in 0..<fling.flings_up {
            dragEnd.press(forDuration: fling.hold_s, thenDragTo: dragStart, withVelocity: velocity, thenHoldForDuration: 0)
            Thread.sleep(forTimeInterval: fling.pause_s)
        }
    }
}
