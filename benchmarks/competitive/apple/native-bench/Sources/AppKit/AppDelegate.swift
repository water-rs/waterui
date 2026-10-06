// Competitive benchmark — AppKit contestant (macOS), workloads W1–W6.
import AppKit

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

func rowColor(_ i: Int) -> NSColor {
    [
        NSColor(red: 0x3B / 255, green: 0x82 / 255, blue: 0xF6 / 255, alpha: 1),
        NSColor(red: 0x10 / 255, green: 0xB9 / 255, blue: 0x81 / 255, alpha: 1),
        NSColor(red: 0xF5 / 255, green: 0x9E / 255, blue: 0x0B / 255, alpha: 1),
        NSColor(red: 0xEF / 255, green: 0x44 / 255, blue: 0x44 / 255, alpha: 1),
        NSColor(red: 0x8B / 255, green: 0x5C / 255, blue: 0xF6 / 255, alpha: 1),
        NSColor(red: 0xEC / 255, green: 0x48 / 255, blue: 0x99 / 255, alpha: 1),
    ][i % 6]
}

func timestamp(_ i: Int) -> String { String(format: "%02d:%02d", (i / 60) % 24, i % 60) }

enum Bench {
    /// `-bench-workload w1|w2|w3|w4|w5|w6` — workload ids are exact
    /// lowercase strings; any other value traps, never silently
    /// measure w1.
    static let workload: String = {
        let raw = UserDefaults.standard.string(forKey: "bench-workload")
        guard let raw, ["w1", "w2", "w3", "w4", "w5", "w6"].contains(raw) else {
            fatalError(
                "missing or unrecognized -bench-workload launch argument "
                    + "(got \(raw ?? "nil")); expected w1..=w6")
        }
        return raw
    }()

    /// `-bench-step N` — the ladder VALUE (200…25600 for w5, 1…64 for
    /// w6), exactly what the runner passes to every contestant. One
    /// launch renders one step; a missing, malformed or off-ladder value
    /// traps.
    static func step(_ ladder: [Int]) -> Int {
        let raw = UserDefaults.standard.string(forKey: "bench-step")
        guard let raw, let n = Int(raw), ladder.contains(n) else {
            fatalError(
                "missing or off-ladder -bench-step (got \(raw ?? "nil")); "
                    + "expected one of \(ladder)")
        }
        return n
    }
}

/// Darwin-notification handshake: the ready post proves the launch
/// argument reached the app. Scrolling is driven from outside the app
/// by OS-level input (the host posts CGEvent scroll-wheel detents into
/// the window); the driver owns cell end — apps never post it.
enum BenchNotify {

    /// Debug trail for the handshake: XCUITest's launch context once left a
    /// check-token poll permanently asleep (register returned OK, posts
    /// never observed), so arm/fire/ack are logged to tmp/bench-app.log —
    /// the file a failure report reads instead of reproducing a deadlock.
    static func dbg(_ msg: String) {
        let line = "\(Date().timeIntervalSince1970) \(msg)\n"
        FileHandle.standardError.write(Data(line.utf8))
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("bench-app.log")
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile(); h.write(Data(line.utf8)); try? h.close()
        } else {
            try? Data(line.utf8).write(to: url)
        }
    }

    /// Posts `dev.bench.ready.<bundle-id>.<W>` when the workload view
    /// first appears — the readiness point every contestant shares. The
    /// runner waits for this post instead of a deep AX query (the 10k-row
    /// feed's accessibility tree takes minutes to materialize).
    static func postReady(_ workload: String) {
        let bid = Bundle.main.bundleIdentifier ?? "unknown"
        notify_post("dev.bench.ready.\(bid).\(workload)")
    }

}

final class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!

    func applicationWillFinishLaunching(_ notification: Notification) {
        BenchNotify.dbg("willFinishLaunching")
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        BenchNotify.dbg("didFinishLaunching start")
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1280, height: 800),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered, defer: false)
        window.title = "Bench AppKit"
        let content: NSViewController
        switch Bench.workload {
        case "w2": content = FeedViewController()
        case "w3": content = MotionViewController()
        case "w4": content = TextBenchViewController()
        case "w5": content =
            MotionViewController(count: Bench.step(
                [200, 400, 800, 1600, 3200, 6400, 12800, 25600]))
        case "w6": content =
            FeedViewController(complexity: Bench.step(
                [1, 2, 4, 8, 16, 32, 64]))
        default: content = HelloViewController()
        }
        window.contentViewController = RootViewController(content: content)
        BenchNotify.dbg("vc assigned viewLoaded=\(window.contentView != nil)")
        window.center()
        window.makeKeyAndOrderFront(nil)
        BenchNotify.dbg("window ordered, windows=\(NSApp.windows.count)")
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
}

/// Hosts the workload's view controller and posts readiness when the
/// workload view first appears in the window — the same point the other
/// contestants use (SwiftUI `onAppear`, UIKit `viewDidAppear`, WaterUI
/// `on_appear`).
final class RootViewController: NSViewController {
    private let content: NSViewController
    private var appeared = false

    init(content: NSViewController) {
        self.content = content
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 1280, height: 800))
        addChild(content)
        content.view.frame = view.bounds
        content.view.autoresizingMask = [.width, .height]
        view.addSubview(content.view)
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard !appeared else { return }
        appeared = true
        BenchNotify.postReady(Bench.workload)
    }
}

// MARK: - W1

final class HelloViewController: NSViewController {
    private var count = 0
    private let label = NSTextField(labelWithString: "Count: 0")

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 1280, height: 800))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        label.font = .systemFont(ofSize: 20)
        let button = NSButton(title: "Increment", target: self, action: #selector(bump))
        button.identifier = NSUserInterfaceItemIdentifier("increment-button")
        button.setAccessibilityIdentifier("increment-button")
        let stack = NSStackView(views: [label, button])
        stack.orientation = .vertical
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }

    @objc private func bump() {
        count += 1
        label.stringValue = "Count: \(count)"
    }
}

// MARK: - W2

final class FeedViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let tableView = NSTableView()
    private let scrollView = NSScrollView()
    private let complexity: Int

    init(complexity: Int = 0) {
        self.complexity = complexity
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 1280, height: 800))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        let col = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("feed"))
        tableView.addTableColumn(col)
        tableView.headerView = nil
        tableView.dataSource = self
        tableView.delegate = self
        tableView.usesAutomaticRowHeights = true
        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scrollView)
        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: view.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
    }

    func numberOfRows(in tableView: NSTableView) -> Int { 10_000 }

    func tableView(_ tableView: NSTableView, viewFor column: NSTableColumn?, row: Int) -> NSView? {
        let id = NSUserInterfaceItemIdentifier("feedCell")
        let cell =
            (tableView.makeView(withIdentifier: id, owner: nil) as? FeedCellView)
            ?? {
                let c = FeedCellView()
                c.identifier = id
                return c
            }()
        cell.configure(row, complexity: complexity)
        return cell
    }

}

final class FeedCellView: NSTableCellView {
    private let avatar = NSView()
    private let title = NSTextField(labelWithString: "")
    private let subtitle = NSTextField(labelWithString: "")
    private let time = NSTextField(labelWithString: "")
    private let cellStack = NSStackView()
    // W6's per-row cell group — pooled on the recycled cell view so a
    // rebound row repopulates the same views, never rebuilds them
    private var cells: [NSStackView] = []

    override init(frame: NSRect) {
        super.init(frame: frame)
        avatar.wantsLayer = true
        avatar.layer?.cornerRadius = 20
        title.font = .systemFont(ofSize: 16)
        subtitle.font = .systemFont(ofSize: 13)
        subtitle.textColor = .secondaryLabelColor
        time.font = .systemFont(ofSize: 13)
        time.textColor = .secondaryLabelColor
        let lines = NSStackView(views: [title, subtitle])
        lines.orientation = .vertical
        lines.alignment = .leading
        lines.spacing = 4
        cellStack.orientation = .horizontal
        cellStack.spacing = 4  // cells are separated by 4 (spec)
        // The text column takes the free width, as on every other
        // contestant: avatar · 12 · text · 12 · [cells · 12 ·] time. The
        // cell group is hidden (and so detached, with its spacing) when a
        // row carries no cells — W2 rows lay out exactly like the others.
        lines.setContentHuggingPriority(.defaultLow, for: .horizontal)
        cellStack.setContentHuggingPriority(.required, for: .horizontal)
        time.setContentHuggingPriority(.required, for: .horizontal)
        let row = NSStackView(views: [avatar, lines, cellStack, time])
        row.orientation = .horizontal
        row.detachesHiddenViews = true
        row.spacing = 12
        row.distribution = .fill
        row.translatesAutoresizingMaskIntoConstraints = false
        avatar.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            avatar.widthAnchor.constraint(equalToConstant: 40),
            avatar.heightAnchor.constraint(equalToConstant: 40),
        ])
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(_ i: Int, complexity: Int = 0) {
        avatar.layer?.backgroundColor = rowColor(i).cgColor
        title.stringValue = "Row title \(i)"
        subtitle.stringValue = "Second line of subtitle for item \(i)"
        time.stringValue = timestamp(i)
        while cells.count < complexity {
            let square = NSView()
            square.wantsLayer = true
            square.layer?.cornerRadius = 14 * 0.3  // radius ratio 0.3
            square.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([
                square.widthAnchor.constraint(equalToConstant: 14),
                square.heightAnchor.constraint(equalToConstant: 14),
            ])
            let cap = NSTextField(labelWithString: "")
            cap.font = .systemFont(ofSize: 12)
            let pair = NSStackView(views: [square, cap])
            pair.orientation = .vertical
            pair.alignment = .centerX
            cells.append(pair)
            cellStack.addArrangedSubview(pair)
        }
        while cells.count > complexity {
            let extra = cells.removeLast()
            cellStack.removeArrangedSubview(extra)
            extra.removeFromSuperview()
        }
        cellStack.isHidden = complexity == 0
        for (j, pair) in cells.enumerated() {
            (pair.arrangedSubviews[0]).layer?.backgroundColor =
                rowColor(i + j).cgColor
            (pair.arrangedSubviews[1] as? NSTextField)?.stringValue = "c\(j)"
        }
    }
}

// MARK: - W3

final class MotionViewController: NSViewController {
    private let fieldW: CGFloat = 720
    private let fieldH: CGFloat = 440
    private var field: NSView!
    private let count: Int

    init(count: Int = 200) {
        self.count = count
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        // The window is the spec's 1280×800; the 720×440 field is a
        // subview centred in it (desktop placement, WORKLOADS.md) — it
        // does not shrink the window or hug a corner.
        view = NSView(frame: NSRect(x: 0, y: 0, width: 1280, height: 800))
        view.wantsLayer = true
        field = NSView(
            frame: NSRect(
                x: (1280 - fieldW) / 2, y: (800 - fieldH) / 2,
                width: fieldW, height: fieldH))
        field.autoresizingMask = [
            .minXMargin, .maxXMargin, .minYMargin, .maxYMargin,
        ]
        view.addSubview(field)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        for i in 0..<count {
            var rng = XorShift(seed: 0xD1B5_4A32_D192_ED03 ^ UInt64(i) &* 0x2545_F491_4F6C_DD1D)
            let v = NSView(
                frame: NSRect(
                    x: rng.next() * (fieldW - 40), y: rng.next() * (fieldH - 40),
                    width: 40, height: 40))
            v.wantsLayer = true
            v.layer?.backgroundColor = rowColor(i).cgColor
            v.layer?.cornerRadius = 10
            // Rotation must pivot on the rect centre: a layer-backed
            // NSView anchors its layer at the lower-left corner, so the
            // anchor and position are set explicitly before transforms.
            v.layer?.anchorPoint = CGPoint(x: 0.5, y: 0.5)
            v.layer?.position = CGPoint(
                x: v.frame.midX, y: v.frame.midY)
            let initRot = rng.next() * .pi * 2
            v.layer?.setAffineTransform(
                CGAffineTransform(rotationAngle: initRot))
            v.alphaValue = 0.3 + rng.next() * 0.7
            field.addSubview(v)
            animate(v, index: i, initialRot: initRot)
        }
    }

    private func animate(_ v: NSView, index: Int, initialRot: Double) {
        let duration = 1.2 + Double(index % 5) * 0.2
        var rng = XorShift(seed: 0x9E37_79B9_7F4A_7C15 ^ UInt64(index) &* 0xBF58_476D_1CE4_E5B9)
        // The rotation channel animates through the layer transform, and
        // the model keeps pace with the animation: each segment's
        // fromValue is where the last segment ended and the model
        // transform is advanced to the target — no snapback between
        // retargets. Drive order per retarget: x, y, rotation, opacity.
        var currentRot = initialRot
        func step() {
            let tx = rng.next() * (self.fieldW - 40)
            let ty = rng.next() * (self.fieldH - 40)
            let rot = rng.next() * .pi * 2
            let alpha = 0.3 + rng.next() * 0.7
            NSAnimationContext.runAnimationGroup({ ctx in
                ctx.duration = duration
                ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                ctx.allowsImplicitAnimation = true
                v.animator().frame.origin = NSPoint(x: tx, y: ty)
                v.animator().alphaValue = alpha
            }, completionHandler: {
                DispatchQueue.main.async { step() }
            })
            let spin = CABasicAnimation(keyPath: "transform.rotation.z")
            spin.fromValue = currentRot
            spin.toValue = rot
            spin.duration = duration
            spin.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            v.layer?.setAffineTransform(
                CGAffineTransform(rotationAngle: rot))
            v.layer?.add(spin, forKey: "spin")
            currentRot = rot
        }
        step()
    }
}

// MARK: - W4

final class TextBenchViewController: NSViewController {
    // Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt.
    private let paragraphs: [String] = [
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

    private let scrollView = NSScrollView()

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 1280, height: 800))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        // All 50 paragraphs laid out eagerly — layout cost is measured.
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        for i in 0..<50 {
            let l = NSTextField(wrappingLabelWithString: paragraphs[i % paragraphs.count])
            l.maximumNumberOfLines = 0
            l.font = .systemFont(ofSize: 16)
            l.translatesAutoresizingMaskIntoConstraints = false
            // paragraph padding horizontal 16, vertical 10 (spec)
            let wrap = NSView()
            wrap.addSubview(l)
            NSLayoutConstraint.activate([
                l.leadingAnchor.constraint(equalTo: wrap.leadingAnchor, constant: 16),
                l.trailingAnchor.constraint(equalTo: wrap.trailingAnchor, constant: -16),
                l.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 10),
                l.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -10),
            ])
            stack.addArrangedSubview(wrap)
        }
        let doc = NSView()
        doc.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: doc.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: doc.trailingAnchor),
            stack.topAnchor.constraint(equalTo: doc.topAnchor),
            stack.bottomAnchor.constraint(equalTo: doc.bottomAnchor),
        ])
        scrollView.documentView = doc
        scrollView.hasVerticalScroller = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scrollView)
        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: view.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            doc.widthAnchor.constraint(equalTo: scrollView.widthAnchor),
        ])
}
}
