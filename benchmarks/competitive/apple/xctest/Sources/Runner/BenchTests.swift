// Shared UI-test harness for the competitive benchmark.
// Drives one workload of one contestant per test run, configured through
// environment variables injected into the .xctestrun EnvironmentVariables:
//   BENCH_BUNDLE_ID  — bundle identifier of the app under test
//   BENCH_WORKLOAD   — W1 | W2 | W3 | W4 | W5 | W6
//   BENCH_DRIVE      — "swipe" (coordinate drags on the window, iOS
//                      device/macOS), "wheel" (host-side CGEvent scroll
//                      into the window — ios-sim/AppKit; the runner's own
//                      process drives it, started by the `dev.bench.begin`
//                      post inside the measure block and ended by its
//                      `dev.bench.done`), or "auto" (capacity-ladder
//                      pacing only: W5/W6 — the app walks its step list on
//                      begin/step/done; scrolling itself is never driven
//                      in-app)
//   BENCH_DURATION   — measurement seconds for W3 (default 12)
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
//   -bench-workload <W1|W2|W3|W4|W5|W6>
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
    private var workload: String = "W1"
    private var drive: String = "swipe"

    override func setUpWithError() throws {
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        bundleID = env["BENCH_BUNDLE_ID"] ?? ""
        try XCTSkipIf(bundleID.isEmpty, "BENCH_BUNDLE_ID not set")
        workload = env["BENCH_WORKLOAD"] ?? "W1"
        drive = env["BENCH_DRIVE"] ?? "swipe"
        app = XCUIApplication(bundleIdentifier: bundleID)
        app.launchArguments = [
            "-bench-workload", workload,
        ]
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
            var fired = false
            let obs = NotificationCenter.default.addObserver(
                forName: ProcessInfo.thermalStateDidChangeNotification,
                object: nil, queue: nil) { _ in fired = true }
            // poll the flag briefly — the notification is the wake-up,
            // the flag join is the deadline discipline
            let wake = Date().addingTimeInterval(min(30, budget))
            while !fired && Date() < wake && Date() < deadline {
                Thread.sleep(forTimeInterval: 0.2)
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
        if ["W2", "W4", "W6"].contains(workload),
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

    private var readyToken: Int32 = 0
    private var readyRegistered = false

    /// Registers `dev.bench.ready.<bundle-id>.<W>` BEFORE app.launch and
    /// drains the latched flag: Darwin notify flags persist across app
    /// launches, so registering after launch cannot distinguish this
    /// launch's post from an identical earlier cell's stale latch — an
    /// app whose delegate never runs (a nibless @main AppKit binary once
    /// did exactly that) would still read "ready" and measure a dead app.
    /// Draining post-registration, pre-launch leaves only a fresh post
    /// able to satisfy the check.
    private func registerWorkloadReady() {
        let name = "dev.bench.ready.\(bundleID).\(workload)"
        guard notify_register_check(name, &readyToken)
                == UInt32(NOTIFY_STATUS_OK)
        else {
            XCTFail("notify_register_check(\(name)) failed")
            return
        }
        readyRegistered = true
        var fired: Int32 = 0
        notify_check(readyToken, &fired)  // consume a stale latched flag
        if fired != 0 { dbg("drained stale ready flag") }
    }

    /// The app posts `dev.bench.ready.<bundle-id>.<W>` over Darwin notify
    /// once it has resolved its -bench-workload argument; absence of the
    /// post means the harness args never reached it and the run is invalid
    /// rather than a W1 measurement.
    private func assertWorkloadReady() {
        let name = "dev.bench.ready.\(bundleID).\(workload)"
        guard readyRegistered else { XCTFail("ready token unregistered"); return }
        let deadline = Date().addingTimeInterval(60)
        var fired: Int32 = 0
        while Date() < deadline {
            notify_check(readyToken, &fired)
            if fired != 0 { break }
            Thread.sleep(forTimeInterval: 0.2)
        }
        notify_cancel(readyToken)
        readyRegistered = false
        XCTAssertTrue(
            fired != 0,
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

        let duration = Double(
            ProcessInfo.processInfo.environment["BENCH_DURATION"] ?? "12") ?? 12

        // device state read on-device per row (devicectl cannot report
        // thermal/fps); the measure-window markers let the host slice
        // its CPU sampler to exactly the measure block
        recordDeviceState()
        dbg("measure-begin")
        measure(metrics: baseMetrics(), options: measureOptions) {
            switch workload {
            case "W2", "W4", "W5", "W6":
                driveScroll(duration: duration)
            case "W3":
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
        var tok: Int32 = 0
        guard notify_register_check("dev.bench.recorder", &tok)
                == UInt32(NOTIFY_STATUS_OK)
        else {
            XCTFail("notify_register_check(dev.bench.recorder) failed")
            return
        }
        var fired: Int32 = 0
        notify_check(tok, &fired)  // drain a stale latch
        let sentinel = FileManager.default.temporaryDirectory
            .appendingPathComponent("bench-recorder-go")
        try? FileManager.default.removeItem(at: sentinel)
        let deadline = Date().addingTimeInterval(300)
        while Date() < deadline {
            fired = 0
            notify_check(tok, &fired)
            if fired != 0 { break }
            if FileManager.default.fileExists(atPath: sentinel.path) {
                fired = 1
                break
            }
            Thread.sleep(forTimeInterval: 0.2)
        }
        notify_cancel(tok)
        dbg("recorder-go fired=\(fired)")
        XCTAssertTrue(fired != 0,
                      "host never armed recorders (dev.bench.recorder)")
    }

    // MARK: - Driving

    /// Same fling sequence for every contestant (../WORKLOADS.md): 8
    /// flings down then 2 back up. `swipe` = coordinate drags; `wheel` =
    /// the host's own process posts CGEvent scroll-wheel detents into the
    /// window (iOS Simulator and AppKit, where gesture synthesis either
    /// stalls on a timed-out AX query or has no swipeable hit target) —
    /// the test only opens the measure window with `dev.bench.begin` and
    /// waits for the host driver's `dev.bench.done`; `auto` paces the
    /// W5/W6 capacity ladder inside the app — on W6 it also performs the
    /// same coordinate drags during each hold.
    private func driveScroll(duration: Double) {
        if drive == "wheel" {
            // The host driver's program is the identical fling protocol;
            // `begin` opens its window, `done` closes the wait. Posts are
            // reposted until a done is seen — the driver may arm after
            // the first post.
            var doneToken: Int32 = 0
            guard notify_register_check("dev.bench.done", &doneToken)
                    == UInt32(NOTIFY_STATUS_OK)
            else {
                XCTFail("wheel drive: notify_register_check failed")
                return
            }
            var fired: Int32 = 0
            _ = notify_check(doneToken, &fired)  // flush stale latch
            let deadline = Date().addingTimeInterval(max(600, duration * 2))
            var lastPost = Date.distantPast
            while Date() < deadline {
                if Date().timeIntervalSince(lastPost) > 5 {
                    notify_post("dev.bench.begin")
                    lastPost = Date()
                }
                fired = 0
                _ = notify_check(doneToken, &fired)
                if fired != 0 { break }
                Thread.sleep(forTimeInterval: 0.2)
            }
            notify_cancel(doneToken)
            XCTAssertTrue(
                fired != 0,
                "wheel drive: host driver never posted 'dev.bench.done'")
            return
        }
        if drive == "auto" {
            // A workload can stall the app's accessibility server for tens
            // of seconds while it materializes (the 10k-row feed), and any
            // AX query that times out records a test failure — so the
            // begin/done handshake goes over Darwin notifications, which
            // need no AX and are identical on simulator, device and macOS.
            var doneToken: Int32 = 0
            var ackToken: Int32 = 0
            var stepToken: Int32 = 0
            guard notify_register_check("dev.bench.done", &doneToken)
                    == UInt32(NOTIFY_STATUS_OK),
                  notify_register_check("dev.bench.ack", &ackToken)
                    == UInt32(NOTIFY_STATUS_OK),
                  notify_register_check("dev.bench.step", &stepToken)
                    == UInt32(NOTIFY_STATUS_OK)
            else {
                XCTFail("auto drive: notify_register_check failed")
                return
            }
            var fired: Int32 = 0
            var acked: Int32 = 0
            var stepFired: Int32 = 0
            // Flush stale done/ack/step flags left over from an earlier run.
            var st = notify_check(doneToken, &fired)
            _ = notify_check(ackToken, &acked)
            _ = notify_check(stepToken, &stepFired)
            dbg("done/ack registered token=\(doneToken)/\(ackToken) flush=\(st) d=\(fired) a=\(acked)")
            // `begin` is re-posted every 5 s only until the app acks that it
            // started — otherwise the flag stays latched and the backlog of
            // extra posts makes the re-armed app replay its program after
            // this window has already measured it.
            // Capacity ladders can stretch far past their nominal length:
            // at the top steps a bridged-runtime app (flutter, rn) starves
            // its own timer callbacks, so `dev.bench.done` lands minutes
            // late — and on a saturated isolate the ladder's *start* can
            // lag the measure block by minutes too. The bound therefore
            // tracks liveness, not wall time: every `dev.bench.step` post
            // (emitted by each contestant's logStep) extends the deadline
            // for another 3 min, so the wait only fails if the program
            // itself has gone silent — while a plain W2/W4 auto program,
            // which posts no steps, still gets the fixed 600 s bound.
            // The pre-ladder lag gets a wide bound too: the app's native
            // side acks `begin` promptly, but a bridged runtime (flutter
            // debug) can starve its isolate for minutes before it consumes
            // the signal — observed ~10 min on this VM. 1200 s covers that
            // without masking a genuinely dead program (steps extend past
            // it regardless).
            // Capacity workloads emit a step post per ladder rung; a
            // step only extends the deadline — the cell completes on the
            // app's own `dev.bench.done` post. A program that posted
            // every step but never `done` is a wedged contestant, not a
            // measured one: counting ladder-shape as completion would
            // let a dead program pass.
            let deadline0 = Date().addingTimeInterval(max(1200, duration * 4))
            var deadline = deadline0
            var lastPost = Date.distantPast
            var sawAck = false
            var ticks = 0
            var stepCount = 0
            // W6 is a scrolling cell: the shared fling protocol runs from
            // here — coordinate drags interleaved with the handshake wait,
            // 8 down then 2 up, repeated for the ladder's duration.
            var flingIndex = 0
            #if os(macOS)
            let w6Anchor: XCUIElement = app.windows.firstMatch
            #else
            let w6Anchor: XCUIElement = app
            #endif
            let w6Down0 = w6Anchor.coordinate(
                withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
            let w6Down1 = w6Anchor.coordinate(
                withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15))
            while Date() < deadline {
                if workload == "W6" {
                    if flingIndex < 8 {
                        w6Down0.press(forDuration: 0.02,
                                      thenDragTo: w6Down1)
                    } else {
                        w6Down1.press(forDuration: 0.02,
                                      thenDragTo: w6Down0)
                    }
                    flingIndex = (flingIndex + 1) % 10
                    Thread.sleep(forTimeInterval: 0.35)
                }
                fired = 0
                st = notify_check(doneToken, &fired)
                if fired != 0 { break }
                stepFired = 0
                _ = notify_check(stepToken, &stepFired)
                if stepFired != 0 {
                    stepCount += 1
                    deadline = Date().addingTimeInterval(180)
                }
                acked = 0
                _ = notify_check(ackToken, &acked)
                // notify_check consumes the flag: latch the first observed
                // ack so reposts stop permanently once the app has started
                // (a repost after ack would latch on the app's re-armed
                // token and replay the program inside a later window).
                if acked != 0 { sawAck = true }
                if !sawAck && Date().timeIntervalSince(lastPost) > 5 {
                    notify_post("dev.bench.begin")
                    lastPost = Date()
                    dbg("posted begin tick=\(ticks) check status=\(st)")
                }
                ticks += 1
                Thread.sleep(forTimeInterval: 0.2)
            }
            dbg("wait loop exited fired=\(fired) sawAck=\(sawAck) steps=\(stepCount) ticks=\(ticks)")
            notify_cancel(doneToken)
            notify_cancel(ackToken)
            notify_cancel(stepToken)
            XCTAssertTrue(
                fired != 0,
                "auto drive: app never posted 'dev.bench.done'")
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
        let dragStart = dragAnchor.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
        let dragEnd = dragAnchor.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15))
        let pause: TimeInterval = 0.35
        for _ in 0..<8 {
            dragStart.press(forDuration: 0.02, thenDragTo: dragEnd)
            Thread.sleep(forTimeInterval: pause)
        }
        for _ in 0..<2 {
            dragEnd.press(forDuration: 0.02, thenDragTo: dragStart)
            Thread.sleep(forTimeInterval: pause)
        }
    }
}
