// Competitive benchmark — AppKit contestant (macOS), workloads W1–W4.
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
    /// `-bench-workload w1|w2|w3|w4` — workload ids are exact lowercase
    /// strings; any other value traps, never silently measure w1.
    static let workload: String = {
        let raw = UserDefaults.standard.string(forKey: "bench-workload")
        guard let raw, ["w1", "w2", "w3", "w4"].contains(raw) else {
            fatalError(
                "missing or unrecognized -bench-workload launch argument "
                    + "(got \(raw ?? "nil")); expected w1|w2|w3|w4")
        }
        BenchNotify.postReady(raw)
        return raw
    }()
}

/// Darwin-notification handshake kept for call-site symmetry with the
/// other Apple contestants — AppKit runs W1–W4 only, none of which arm
/// it. Scrolling is driven from outside the app by OS-level input (the
/// host posts CGEvent scroll-wheel detents into the window).
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

    static func postDone() { notify_post("dev.bench.done") }

    /// Posts `dev.bench.ready.<bundle-id>.<W>` once the workload argument
    /// has resolved — the runner waits for this post to confirm the
    /// argument arrived, instead of a deep AX query (the 10k-row feed's
    /// accessibility tree takes minutes to materialize).
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
        switch Bench.workload {
        case "w2": window.contentViewController = FeedViewController()
        case "w3": window.contentViewController = MotionViewController()
        case "w4": window.contentViewController = TextBenchViewController()
        default: window.contentViewController = HelloViewController()
        }
        BenchNotify.dbg("vc assigned viewLoaded=\(window.contentView != nil)")
        // The runner asserts this identifier after launch. A plain NSView
        // never enters the AX tree, so the id rides on a dedicated element.
        let marker = NSTextField(
            labelWithString: "bench-workload-\(Bench.workload)")
        marker.frame = NSRect(x: 0, y: 0, width: 1, height: 1)
        marker.font = .systemFont(ofSize: 1)
        marker.setAccessibilityIdentifier("bench-workload-\(Bench.workload)")
        window.contentView?.addSubview(marker)
        window.center()
        window.makeKeyAndOrderFront(nil)
        BenchNotify.dbg("window ordered, windows=\(NSApp.windows.count)")
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
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
        cell.configure(row)
        return cell
    }

}

final class FeedCellView: NSTableCellView {
    private let avatar = NSView()
    private let title = NSTextField(labelWithString: "")
    private let subtitle = NSTextField(labelWithString: "")
    private let time = NSTextField(labelWithString: "")

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
        let row = NSStackView(views: [avatar, lines, NSView(), time])
        row.orientation = .horizontal
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

    func configure(_ i: Int) {
        avatar.layer?.backgroundColor = rowColor(i).cgColor
        title.stringValue = "Row title \(i)"
        subtitle.stringValue = "Second line of subtitle for item \(i)"
        time.stringValue = timestamp(i)
    }
}

// MARK: - W3

final class MotionViewController: NSViewController {
    private let fieldW: CGFloat = 720
    private let fieldH: CGFloat = 440
    private var field: NSView!

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
        for i in 0..<200 {
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
            v.layer?.setAffineTransform(
                CGAffineTransform(rotationAngle: rng.next() * .pi * 2))
            v.alphaValue = 0.3 + rng.next() * 0.7
            field.addSubview(v)
            animate(v, index: i)
        }
    }

    private func animate(_ v: NSView, index: Int) {
        let duration = 1.2 + Double(index % 5) * 0.2
        var rng = XorShift(seed: 0x9E37_79B9_7F4A_7C15 ^ UInt64(index) &* 0xBF58_476D_1CE4_E5B9)
        func step() {
            NSAnimationContext.runAnimationGroup({ ctx in
                ctx.duration = duration
                ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                ctx.allowsImplicitAnimation = true
                v.animator().frame.origin = NSPoint(
                    x: rng.next() * (self.fieldW - 40), y: rng.next() * (self.fieldH - 40))
                v.animator().alphaValue = 0.3 + rng.next() * 0.7
            }, completionHandler: {
                DispatchQueue.main.async { step() }
            })
            // Rotation via layer transform — the anchor point is the
            // layer centre, so the rect spins about its middle.
            let rot = CABasicAnimation(keyPath: "transform.rotation.z")
            rot.toValue = rng.next() * .pi * 2
            rot.duration = duration
            rot.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            rot.isRemovedOnCompletion = false
            rot.fillMode = .forwards
            v.layer?.add(rot, forKey: "spin")
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
