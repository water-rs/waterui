// Competitive benchmark app — SwiftUI contestant (iOS).
// Workloads W1–W6 per water-rs/waterui#1262 (canonical spec:
// benchmarks/competitive/WORKLOADS.md); workload selected with the launch
// argument `-bench-workload W1|W2|W3|W4|W5|W6`, read through
// NSUserDefaults' NSArgumentDomain. Missing or unrecognized values trap.
// Scrolling is driven from outside the app by OS-level input — the app
// never scrolls itself. Capacity workloads are paced one launch per step
// (`-bench-step N`); the host driver ends every cell on its own
// schedule — apps post readiness only, never completion.

import SwiftUI

/// Darwin-notification readiness handshake. AX queries cannot carry
/// it: a workload can stall the app's accessibility server for tens of
/// seconds while it materializes (the 10k-row feed), and a timed-out
/// query fails the test instead of driving it.
enum BenchNotify {
    /// Posts `dev.bench.ready.<bundle-id>.<w>` when the workload view
    /// first appears — the readiness point every contestant shares. The
    /// runner waits for this post instead of a deep AX query (the 10k-row
    /// feed's accessibility tree takes minutes to materialize).
    static func postReady(_ workload: String) {
        let bid = Bundle.main.bundleIdentifier ?? "unknown"
        notify_post("dev.bench.ready.\(bid).\(workload)")
    }
}

enum Workload: String {
    case hello = "w1", feed = "w2", motion = "w3", text = "w4"
    case motionCapacity = "w5", feedCapacity = "w6"

    static func current() -> Workload {
        let raw = UserDefaults.standard.string(forKey: "bench-workload")
        guard let raw, let w = Workload(rawValue: raw) else {
            fatalError(
                "missing or unrecognized -bench-workload launch argument "
                    + "(got \(raw ?? "nil")); expected w1..=w6")
        }
        return w
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

/// `-bench-step N` — one launch renders one capacity step; a missing,
/// malformed or off-ladder step traps (w5/w6 only).
func benchStep(_ ladder: [Int]) -> Int {
    let raw = UserDefaults.standard.integer(forKey: "bench-step")
    guard raw != 0, ladder.contains(raw) else {
        fatalError(
            "missing or off-ladder -bench-step (got \(raw)); "
                + "expected one of \(ladder)")
    }
    return raw
}

func timestamp(for index: Int) -> String {
    String(format: "%02d:%02d", (index / 60) % 24, index % 60)
}

@main
struct BenchApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
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
        // readiness = the workload view's first appearance, the point
        // every contestant posts at
        .onAppear { BenchNotify.postReady(workload.rawValue) }
    }
}

// MARK: - W1 Hello

struct HelloView: View {
    @State private var count = 0
    var body: some View {
        VStack(spacing: 16) {
            Text("Count: \(count)").font(.system(size: 20))
            Button("Increment") { count += 1 }
                .accessibilityIdentifier("increment-button")
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - W2 Feed

struct FeedView: View {
    var body: some View {
        List(0..<10_000, id: \.self) { i in
            HStack(spacing: 12) {
                Circle().fill(rowColors[i % 6]).frame(width: 40, height: 40)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Row title \(i)").font(.system(size: 16))
                    Text("Second line of subtitle for item \(i)")
                        .font(.system(size: 13)).foregroundStyle(.secondary)
                }
                Spacer()
                Text(timestamp(for: i))
                    .font(.system(size: 13)).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
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
        // Init stream: x, y, rotation, opacity draws in order — no
        // discarded draws (WORKLOADS.md).
        var initRng = XorShift(seed: 0xD1B54A32D192ED03 ^ UInt64(index) &* 0x2545F4914F6CDD1D)
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
                // First retarget at t=0 — targets animate for `duration`,
                // then the next draw arrives on the same cadence.
                while !Task.isCancelled {
                    let d = duration
                    withAnimation(.easeInOut(duration: d)) {
                        x = rng.next() * (fieldW - 40)
                        y = rng.next() * (fieldH - 40)
                        rot = rng.next() * 360
                        op = 0.3 + rng.next() * 0.7
                    }
                    try? await Task.sleep(nanoseconds: UInt64(d * 1_000_000_000))
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
        // field placement (mobile layout): pinned to the top of the
        // content area with a 16-point inset
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .padding(.top, 16)
    }
}

// MARK: - W4 Text

// Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt, embedded
// (an app cannot read the suite's file at runtime).
let paragraphs: [String] = [
    "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳過懶惰的狗。🦊🐶 Packing my box with five dozen liquor jugs.",
    "WaterUI renders native widgets from a single Rust view tree. 水のインターフェースはネイティブウィジェットを描画する。🌊",
    "Almost all programming can be viewed as state management. 几乎所有的编程都可以视为状态管理。📚 Signals flow through the graph.",
    "Sphinx of black quartz, judge my vow. 黒い水晶のスフィンクス、私の誓いを裁け。🗻 Typography is the visual component of the written word.",
    "How vexingly quick daft zebras jump! 빠른 얼룩말이 얼마나 성가시게 뛰는가! 🦓 The first principle is that you must not fool yourself.",
    "Bright vixens jump; dozy fowl quack. 밝은 여우가 뛰고 졸린 새가 꽥꽥 운다. 🐦 Rendering pipelines measure progress in milliseconds per frame.",
    "ベンチマークが正直であれば最適化も正直になる。Benchmarks that are honest make optimisation honest. 📏",
    "Two driven jocks help fax my big quiz. 두 명의 조키가 내 큰 퀴즈를 팩스로 보내는 것을 돕는다. 🌲 Lazily built lists keep memory flat.",
    "The five boxing wizards jump quickly. 五個拳擊巫師跳得很快。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
    "Jackdaws love my big sphinx of quartz. 寒鸦喜欢我巨大的石英斯芬克斯。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
]

/// All 50 paragraphs are laid out eagerly inside one ScrollView — layout
/// cost is part of the measurement, so nothing may be lazy.
struct TextBenchView: View {
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(0..<50, id: \.self) { i in
                    Text(paragraphs[i % paragraphs.count])
                        .font(.system(size: 16))
                        .padding(.horizontal, 16).padding(.vertical, 10)
                }
            }
        }
    }
}

// MARK: - W5 Motion capacity

/// W3's scene at one pinned ladder step (200…25600) — one launch renders
/// one step; the host driver ends the cell on its own schedule.
struct MotionCapacityView: View {
    private static let steps = [200, 400, 800, 1600, 3200, 6400, 12800, 25600]
    private let count = benchStep(Self.steps)
    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(0..<count, id: \.self) { MotionRect(index: $0) }
        }
        .frame(width: fieldW, height: fieldH)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .padding(.top, 16)
    }
}

// MARK: - W6 Feed capacity

/// W2's fling program over rows at one pinned complexity (1…64) — one
/// launch renders one step.
struct FeedCapacityView: View {
    private static let steps = [1, 2, 4, 8, 16, 32, 64]
    private let complexity = benchStep(Self.steps)
    var body: some View {
        List(0..<10_000, id: \.self) { i in
            HStack(spacing: 12) {
                Circle().fill(rowColors[i % 6]).frame(width: 40, height: 40)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Row title \(i)").font(.system(size: 16))
                    Text("Second line of subtitle for item \(i)")
                        .font(.system(size: 13)).foregroundStyle(.secondary)
                }
                Spacer()
                // cells separated by 4 horizontally (the outer HStack's
                // 12 is the column gap to the timestamp, not cell pitch)
                HStack(spacing: 4) {
                    ForEach(0..<complexity, id: \.self) { j in
                        VStack {
                            RoundedRectangle(cornerRadius: 4)
                                .fill(rowColors[(i + j) % 6])
                                .frame(width: 14, height: 14)
                            Text("c\(j)").font(.system(size: 12))
                        }
                    }
                }
                Text(timestamp(for: i))
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
        }
    }
}
