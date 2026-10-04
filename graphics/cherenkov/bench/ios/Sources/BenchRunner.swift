import Foundation
import OSLog

private let logger = Logger(subsystem: "dev.cherenkov", category: "bench")

/// One finished run: its arguments and exit code, as recorded in
/// `done.json`, with the device thermal state bracketing the run
/// (`nominal`, `fair`, `serious`, `critical`, or `unknown`).
struct BenchRunResult {
    let args: [String]
    let exitCode: Int32
    let thermalBefore: String
    let thermalAfter: String
}

/// Receives bench progress on the main thread.
protocol BenchRunnerDelegate: AnyObject {
    /// `index` (zero-based) of `total` is about to run `args`.
    func benchRunner(_ runner: BenchRunner, didStartRun index: Int, of total: Int, args: [String])
    /// Every run finished; `results` is in `done.json` order.
    func benchRunner(_ runner: BenchRunner, didFinishWithResults results: [BenchRunResult])
    /// The suite could not run to completion; `error` says why
    /// (`bench-args.json` missing or invalid, `Documents/out` or
    /// `done.json` unwritable, ...).
    func benchRunner(_ runner: BenchRunner, didFailWithError error: String)
}

/// Runs every argument list in `Documents/bench-args.json` through
/// `cherenkov_bench_run`, in order, and records each run's exit code in
/// `Documents/out/done.json`.
///
/// `bench-args.json` is `{"run_id": "...", "runs": [[...]]}`. That
/// `run_id` is written to `Documents/thermal.json` before `out` is
/// reset, and to `done.json` when the launch finishes.
struct BenchRunner {
    /// Receives progress on the main thread.
    let delegate: BenchRunnerDelegate?

    init(delegate: BenchRunnerDelegate? = nil) {
        self.delegate = delegate
    }

    /// `ProcessInfo.thermalState` as the words the device gate uses.
    static func thermalStateWord() -> String {
        switch ProcessInfo.processInfo.thermalState {
        case .nominal:
            return "nominal"
        case .fair:
            return "fair"
        case .serious:
            return "serious"
        case .critical:
            return "critical"
        @unknown default:
            return "unknown"
        }
    }

    /// The overall exit code: the first non-zero run's, else 0.
    func runAll() -> Int32 {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let outDir = documents.appendingPathComponent("out", isDirectory: true)
        let argsURL = documents.appendingPathComponent("bench-args.json")
        // Parsed before thermal.json: the run id has to be the one the
        // host wrote, and a missing file must not invent one.
        guard let benchArgs = Self.loadBenchArgs(from: argsURL) else {
            if case let .failure(message) = Self.resetOut(outDir) {
                report { $0.benchRunner(self, didFailWithError: message) }
                return 1
            }
            _ = Self.writeDone(["error": "bench-args.json missing or invalid"], to: outDir)
            report { $0.benchRunner(self, didFailWithError: "bench-args.json missing or invalid") }
            return 1
        }
        let runId = benchArgs.runId
        let argLists = benchArgs.runs
        // Outside `out`: the reset below deletes `out`, and the driver
        // reads this before the measurement exists.
        let thermal = Self.thermalStateWord()
        let thermalURL = documents.appendingPathComponent("thermal.json")
        guard Self.writeJSON(["state": thermal, "run_id": runId], to: thermalURL) else {
            logger.error("cannot write \(thermalURL.path, privacy: .public)")
            return 1
        }
        logger.info("thermal \(thermal, privacy: .public) run \(runId, privacy: .public)")
        if thermal != "nominal" && thermal != "fair" {
            if case .failure = Self.resetOut(outDir) {
                return 1
            }
            _ = Self.writeDone(
                [
                    "error": "thermal \(thermal)",
                    "thermal": thermal,
                    "run_id": runId,
                    "results": [Any](),
                ],
                to: outDir
            )
            report { $0.benchRunner(self, didFailWithError: "thermal \(thermal)") }
            return 1
        }
        // `out` holds this launch's results only: a `done.json` left by an
        // earlier launch would read as this one having finished.
        if case let .failure(message) = Self.resetOut(outDir) {
            report { $0.benchRunner(self, didFailWithError: message) }
            return 1
        }
        // An iOS app launches with cwd `/`; the bench's relative
        // `Documents/...` paths only resolve from the app's home.
        guard FileManager.default.changeCurrentDirectoryPath(NSHomeDirectory()) else {
            logger.error("cannot chdir to \(NSHomeDirectory(), privacy: .public)")
            _ = Self.writeDone(
                ["error": "cannot chdir to app home", "run_id": runId],
                to: outDir
            )
            report { $0.benchRunner(self, didFailWithError: "cannot chdir to app home") }
            return 1
        }
        var results: [BenchRunResult] = []
        var firstFailure: Int32 = 0
        for (index, args) in argLists.enumerated() {
            logger.info("run \(index): \(args.joined(separator: " "), privacy: .public)")
            report { $0.benchRunner(self, didStartRun: index, of: argLists.count, args: args) }
            let logURL = outDir.appendingPathComponent("run-\(index).log")
            let thermalBefore = Self.thermalStateWord()
            let code = Self.invoke(args, logTo: logURL)
            let thermalAfter = Self.thermalStateWord()
            logger.info("run \(index): exit \(code), thermal \(thermalBefore)->\(thermalAfter)")
            results.append(BenchRunResult(
                args: args,
                exitCode: code,
                thermalBefore: thermalBefore,
                thermalAfter: thermalAfter
            ))
            if firstFailure == 0 { firstFailure = code }
        }
        let doneResults = results.map {
            [
                "args": $0.args,
                "exit_code": $0.exitCode,
                "thermal_before": $0.thermalBefore,
                "thermal_after": $0.thermalAfter,
            ] as [String: Any]
        }
        guard Self.writeDone(["run_id": runId, "results": doneResults], to: outDir) else {
            report { $0.benchRunner(self, didFailWithError: "cannot write done.json") }
            return 1
        }
        report { $0.benchRunner(self, didFinishWithResults: results) }
        return firstFailure
    }

    /// Calls `body` with the delegate on the main thread.
    private func report(_ body: @escaping (BenchRunnerDelegate) -> Void) {
        guard let delegate else { return }
        DispatchQueue.main.async { body(delegate) }
    }

    /// Calls `cherenkov_bench_run` with `cherenkov-bench` as `argv[0]`
    /// followed by `args`, with fds 1 and 2 redirected to `logURL` for
    /// the duration of the call and restored afterwards.
    static func invoke(_ args: [String], logTo logURL: URL) -> Int32 {
        var cArgs = (["cherenkov-bench"] + args).map { strdup($0) }
        defer { cArgs.forEach { free($0) } }

        let savedOut = dup(STDOUT_FILENO)
        let savedErr = dup(STDERR_FILENO)
        let log = open(logURL.path, O_WRONLY | O_CREAT | O_TRUNC, 0o644)
        let redirected = savedOut >= 0 && savedErr >= 0 && log >= 0
        if redirected {
            fflush(nil)
            dup2(log, STDOUT_FILENO)
            dup2(log, STDERR_FILENO)
        } else {
            logger.error("log redirect to \(logURL.path, privacy: .public) failed (savedOut=\(savedOut), savedErr=\(savedErr), log=\(log))")
        }
        if log >= 0 { close(log) }

        let code = cArgs.withUnsafeMutableBufferPointer { buffer in
            buffer.baseAddress!.withMemoryRebound(
                to: UnsafePointer<CChar>?.self,
                capacity: buffer.count
            ) { argv in
                cherenkov_bench_run(Int32(buffer.count), argv)
            }
        }

        fflush(nil)
        if redirected {
            dup2(savedOut, STDOUT_FILENO)
            dup2(savedErr, STDERR_FILENO)
        }
        if savedOut >= 0 { close(savedOut) }
        if savedErr >= 0 { close(savedErr) }
        return code
    }

    /// `bench-args.json`: the host's run id and the argument lists.
    struct BenchArgs {
        let runId: String
        let runs: [[String]]
    }

    /// Empties `Documents/out` so a previous launch's `done.json` cannot
    /// be read as this one.
    static func resetOut(_ outDir: URL) -> Result<Void, String> {
        do {
            if FileManager.default.fileExists(atPath: outDir.path) {
                try FileManager.default.removeItem(at: outDir)
            }
            try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)
            return .success(())
        } catch {
            let message = "cannot reset \(outDir.path): \(error.localizedDescription)"
            logger.error("\(message, privacy: .public)")
            return .failure(message)
        }
    }

    /// Parses `{"run_id": "...", "runs": [[String]]}`. An empty run id or
    /// an empty `runs` array is invalid: the host always names the launch.
    static func loadBenchArgs(from url: URL) -> BenchArgs? {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let runId = object["run_id"] as? String,
              !runId.isEmpty,
              let rawRuns = object["runs"] as? [Any],
              !rawRuns.isEmpty
        else {
            logger.error("\(url.lastPathComponent, privacy: .public) missing or not {run_id, runs}")
            return nil
        }
        var runs: [[String]] = []
        runs.reserveCapacity(rawRuns.count)
        for item in rawRuns {
            guard let args = item as? [String] else {
                logger.error("\(url.lastPathComponent, privacy: .public) runs must be arrays of strings")
                return nil
            }
            runs.append(args)
        }
        return BenchArgs(runId: runId, runs: runs)
    }

    @discardableResult
    static func writeJSON(_ object: [String: Any], to url: URL) -> Bool {
        do {
            let data = try JSONSerialization.data(
                withJSONObject: object,
                options: [.prettyPrinted, .sortedKeys]
            )
            try data.write(to: url, options: .atomic)
            return true
        } catch {
            logger.error("cannot write \(url.path, privacy: .public): \(error)")
            return false
        }
    }

    @discardableResult
    static func writeDone(_ object: [String: Any], to outDir: URL) -> Bool {
        let url = outDir.appendingPathComponent("done.json")
        let ok = writeJSON(object, to: url)
        if ok {
            logger.info("wrote \(url.path, privacy: .public)")
        }
        return ok
    }
}
