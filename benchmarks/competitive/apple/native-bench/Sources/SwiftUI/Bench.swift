// Competitive benchmark app — SwiftUI contestant (iOS + macOS).
// Workloads W1–W4 per water-rs/waterui#1262; workload selected with the
// launch argument `-bench-workload W1|W2|W3|W4|W5|W6`, read through
// NSUserDefaults' NSArgumentDomain. Missing or unrecognized values trap.
// `-bench-drive auto` makes scroll workloads run the shared fling program
// themselves, but only once the runner posts the `dev.bench.begin` Darwin
// notification inside its measure block.

import SwiftUI

/// Darwin-notification handshake for the `auto` drive. AX queries cannot
/// carry it: a workload can stall the app's accessibility server for tens
/// of seconds while it materializes (the 10k-row feed), and a timed-out
/// query fails the test instead of driving it. The runner posts
/// `dev.bench.begin` inside its `measure` block and waits for the app to
/// post `dev.bench.done` when the program finishes.
enum BenchNotify {
    private static var token: Int32 = 0
    private static var armed = false

    /// Suspends until the runner posts `dev.bench.begin` (50 ms poll). The
    /// check token is kept alive: each `notify_post` re-fires it, so XCTest
    /// may invoke the measure block more than once per run.
    static func awaitBegin() async {
        if !armed {
            guard notify_register_check("dev.bench.begin", &token)
                    == UInt32(NOTIFY_STATUS_OK)
            else { return }
            var stale: Int32 = 0
            // The first check reports the flag's current state — consume any
            // stale post left over from a previous run.
            notify_check(token, &stale)
            armed = true
        }
        var fired: Int32 = 0
        while fired == 0 {
            notify_check(token, &fired)
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
        notify_post("dev.bench.ack")  // stops the runner's reposts
    }

    static func postDone() { notify_post("dev.bench.done") }

    /// Signals `dev.bench.step` — a capacity-ladder step started. The step
    /// log is what the runner actually slices on; the post is a live
    /// signal for the XCTest driver.
    static func postStep() { notify_post("dev.bench.step") }

    /// Posts `dev.bench.ready.<bundle-id>.<W>` once the workload argument
    /// has resolved — the runner waits for this post to confirm the
    /// argument arrived, instead of a deep AX query (the 10k-row feed's
    /// accessibility tree takes minutes to materialize).
    static func postReady(_ workload: String) {
        let bid = Bundle.main.bundleIdentifier ?? "unknown"
        notify_post("dev.bench.ready.\(bid).\(workload)")
    }

    /// Consumes a begin that latched while a program ran (a repost that
    /// raced the ack). The runner stops posting once it sees done, so a flag
    /// already set at re-arm time is backlog, not a new signal.
    static func discardLatchedBegin() {
        guard armed else { return }
        var fired: Int32 = 0
        notify_check(token, &fired)
    }
}

/// Capacity-ladder step record: `step <k> n=<param> t=<unix-seconds>`
/// appended to tmp/bench-steps.log plus a `dev.bench.step` post — the
/// runner pulls the file and slices its xctrace recording by `t`.
func logBenchStep(_ step: Int, param: Int) {
    let line = String(
        format: "step %d n=%d t=%.3f\n",
        step, param, Date().timeIntervalSince1970)
    let path = NSTemporaryDirectory() + "bench-steps.log"
    if let h = FileHandle(forWritingAtPath: path)
        ?? (FileManager.default.createFile(atPath: path, contents: nil)
            ? FileHandle(forWritingAtPath: path) : nil) {
        h.seekToEndOfFile()
        h.write(Data(line.utf8))
        try? h.close()
    }
    BenchNotify.postStep()
}

enum Workload: String {
    case hello = "W1", feed = "W2", motion = "W3", text = "W4"
    case motionCapacity = "W5", feedCapacity = "W6"

    static func current() -> Workload {
        let raw = UserDefaults.standard.string(forKey: "bench-workload")
        guard let raw, let w = Workload(rawValue: raw) else {
            fatalError(
                "missing or unrecognized -bench-workload launch argument "
                    + "(got \(raw ?? "nil")); expected W1..=W6")
        }
        BenchNotify.postReady(raw)
        return w
    }

    static var autoDrive: Bool {
        let raw = UserDefaults.standard.string(forKey: "bench-drive") ?? "swipe"
        guard raw == "swipe" || raw == "auto" else {
            fatalError(
                "unrecognized -bench-drive value \(raw); expected swipe|auto")
        }
        return raw == "auto"
    }
}

/// Deterministic PRNG so every contestant animates the same sequence.
struct XorShift {
    var state: UInt64
    init(seed: UInt64) { state = seed }
    mutating func next() -> Double {
        state ^= state << 13
        state ^= state >> 7
        state ^= state << 17
        return Double(state % 10_000) / 10_000.0
    }
}

let rowColors: [Color] = [
    Color(hex: 0x3B82F6), Color(hex: 0x10B981), Color(hex: 0xF59E0B),
    Color(hex: 0xEF4444), Color(hex: 0x8B5CF6), Color(hex: 0xEC4899),
]

extension Color {
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255.0,
            green: Double((hex >> 8) & 0xFF) / 255.0,
            blue: Double(hex & 0xFF) / 255.0
        )
    }
}

func timestamp(for index: Int) -> String {
    String(format: "%02d:%02d", (index / 60) % 24, index % 60)
}

@main
struct BenchApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .onAppear {
                    // First-frame marker for the external launch measurement.
                    DispatchQueue.main.async { print("BENCH_READY") }
                }
        }
    }
}

struct ContentView: View {
    let workload = Workload.current()
    var body: some View {
        Group {
            switch workload {
            case .feed: FeedView()
            case .motion: MotionView()
            case .text: TextBenchView()
            case .motionCapacity: MotionCapacityView()
            case .feedCapacity: FeedCapacityView()
            default: HelloView()
            }
        }
        // The runner asserts this identifier after launch.
        .accessibilityIdentifier("bench-workload-\(workload.rawValue)")
    }
}

// MARK: - W1 Hello

struct HelloView: View {
    @State private var count = 0
    var body: some View {
        VStack(spacing: 16) {
            Text("Count: \(count)").font(.title)
            Button("Increment") { count += 1 }
                .accessibilityIdentifier("increment-button")
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - W2 Feed

struct FeedView: View {
    @State private var scrolledID: Int? = 0
    var body: some View {
        List(0..<10_000, id: \.self) { i in
            HStack(spacing: 12) {
                Circle().fill(rowColors[i % 6]).frame(width: 40, height: 40)
                VStack(alignment: .leading) {
                    Text("Row title \(i)")
                    Text("Second line of subtitle for item \(i)")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Text(timestamp(for: i)).font(.caption).foregroundStyle(.secondary)
            }
            .padding(.vertical, 4)
        }
        .scrollPosition(id: $scrolledID)
        .task {
            guard Workload.autoDrive else { return }
            while true {
                await BenchNotify.awaitBegin()
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                // Fling program: 8 bursts to the bottom, 2 back to the top.
                for step in 1...8 {
                    withAnimation(.easeOut(duration: 0.9)) {
                        scrolledID = min(step * 1250, 9_999)
                    }
                    try? await Task.sleep(nanoseconds: 1_150_000_000)
                }
                for step in stride(from: 8, through: 0, by: -4) {
                    withAnimation(.easeOut(duration: 0.9)) {
                        scrolledID = max(step * 1250, 0)
                    }
                    try? await Task.sleep(nanoseconds: 1_150_000_000)
                }
                BenchNotify.postDone()
                BenchNotify.discardLatchedBegin()
            }
        }
    }
}

// MARK: - W3 Motion

private let fieldW: CGFloat = 720
private let fieldH: CGFloat = 440

struct MotionRect: View {
    let index: Int
    @State private var x: CGFloat
    @State private var y: CGFloat
    @State private var rot: Double
    @State private var op: Double
    private let rngSeed: UInt64
    private let duration: Double

    init(index: Int) {
        self.index = index
        var s = XorShift(seed: 0xD1B54A32D192ED03 ^ UInt64(index) &* 0x2545F4914F6CDD1D)
        let seed = s.state
        _ = s.next()
        var initRng = XorShift(seed: seed)
        _x = State(initialValue: initRng.next() * (fieldW - 40))
        _y = State(initialValue: initRng.next() * (fieldH - 40))
        _rot = State(initialValue: initRng.next() * 360)
        _op = State(initialValue: 0.3 + initRng.next() * 0.7)
        rngSeed = 0x9E3779B97F4A7C15 ^ UInt64(index) &* 0xBF58476D1CE4E5B9
        duration = 1.2 + Double(index % 5) * 0.2
    }

    var body: some View {
        RoundedRectangle(cornerRadius: 10)
            .fill(rowColors[index % 6])
            .frame(width: 40, height: 40)
            .rotationEffect(.degrees(rot))
            .opacity(op)
            .position(x: x + 20, y: y + 20)
            .task {
                var rng = XorShift(seed: rngSeed)
                while !Task.isCancelled {
                    let d = duration
                    try? await Task.sleep(nanoseconds: UInt64(d * 1_000_000_000))
                    withAnimation(.easeInOut(duration: d)) {
                        x = rng.next() * (fieldW - 40)
                        y = rng.next() * (fieldH - 40)
                        rot = rng.next() * 360
                        op = 0.3 + rng.next() * 0.7
                    }
                }
            }
    }
}

struct MotionView: View {
    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(0..<200, id: \.self) { MotionRect(index: $0) }
        }
        .frame(width: fieldW, height: fieldH)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

// MARK: - W4 Text

let paragraphs: [String] = [
    "The quick brown fox jumps over the lazy dog. 。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 。🌊 Fine-grained reactivity updates only the widgets that read the value.",
    "Almost all programming can be viewed as state management. ，。📚 Signals flow through the graph and wake the views that observe them.",
    "Sphinx of black quartz, judge my vow. のテキストもぜます。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! ，。🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. ，。🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. ，。🌲 Lazily built lists keep memory flat while content grows without bound.",
    "The five boxing wizards jump quickly. ，。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. ，。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
]

struct TextBenchView: View {
    @State private var scrolledID: Int? = 0
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(0..<50, id: \.self) { i in
                    Text(paragraphs[i % paragraphs.count])
                        .padding(.horizontal, 16).padding(.vertical, 6)
                        .id(i)
                }
            }
        }
        .scrollPosition(id: $scrolledID)
        .task {
            guard Workload.autoDrive else { return }
            while true {
                await BenchNotify.awaitBegin()
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                // Same fling program as W2, over 50 rows: 8 bursts down, 2 back up.
                for step in 1...8 {
                    withAnimation(.easeOut(duration: 0.9)) {
                        scrolledID = min(step * 6, 49)
                    }
                    try? await Task.sleep(nanoseconds: 1_150_000_000)
                }
                for step in stride(from: 8, through: 0, by: -4) {
                    withAnimation(.easeOut(duration: 0.9)) {
                        scrolledID = max(step * 6, 0)
                    }
                    try? await Task.sleep(nanoseconds: 1_150_000_000)
                }
                BenchNotify.postDone()
                BenchNotify.discardLatchedBegin()
            }
        }
    }
}

// MARK: - W5 Motion capacity

/// W3's scene with the rect count doubled per step (200…25600). After
/// `dev.bench.begin` the ladder sets `@State count`, logs the step, holds
/// 5 s, advances; `dev.bench.done` ends it. Same cadence as every other
/// contestant — step boundaries come from bench-steps.log.
struct MotionCapacityView: View {
    private static let steps = [200, 400, 800, 1600, 3200, 6400, 12800, 25600]
    @State private var count = 200
    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(0..<count, id: \.self) { MotionRect(index: $0) }
        }
        .frame(width: fieldW, height: fieldH)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .task {
            guard Workload.autoDrive else { return }
            while true {
                await BenchNotify.awaitBegin()
                for (i, n) in Self.steps.enumerated() {
                    count = n
                    logBenchStep(i, param: n)
                    try? await Task.sleep(nanoseconds: 5_000_000_000)
                }
                BenchNotify.postDone()
                BenchNotify.discardLatchedBegin()
            }
        }
    }
}

// MARK: - W6 Feed capacity

/// W2's fling program over rows with `complexity` nested text+shape
/// children per row (1…64). Two full sweeps per step inside the 5 s hold.
struct FeedCapacityView: View {
    private static let steps = [1, 2, 4, 8, 16, 32, 64]
    @State private var complexity = 1
    @State private var scrolledID: Int? = 0
    var body: some View {
        List(0..<10_000, id: \.self) { i in
            HStack(spacing: 12) {
                Circle().fill(rowColors[i % 6]).frame(width: 40, height: 40)
                VStack(alignment: .leading) {
                    Text("Row title \(i)")
                    Text("Second line of subtitle for item \(i)")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                ForEach(0..<complexity, id: \.self) { j in
                    VStack {
                        RoundedRectangle(cornerRadius: 4)
                            .fill(rowColors[(i + j) % 6])
                            .frame(width: 14, height: 14)
                        Text("c\(j)").font(.caption2)
                    }
                }
                Text(timestamp(for: i)).font(.caption).foregroundStyle(.secondary)
            }
            .padding(.vertical, 4)
        }
        .scrollPosition(id: $scrolledID)
        .task {
            guard Workload.autoDrive else { return }
            while true {
                await BenchNotify.awaitBegin()
                for (i, k) in Self.steps.enumerated() {
                    complexity = k
                    logBenchStep(i, param: k)
                    try? await Task.sleep(nanoseconds: 1_000_000_000)
                    for _ in 0..<2 {
                        withAnimation(.easeOut(duration: 0.9)) { scrolledID = 9_999 }
                        try? await Task.sleep(nanoseconds: 1_000_000_000)
                        withAnimation(.easeOut(duration: 0.9)) { scrolledID = 0 }
                        try? await Task.sleep(nanoseconds: 1_000_000_000)
                    }
                }
                BenchNotify.postDone()
                BenchNotify.discardLatchedBegin()
            }
        }
    }
}
