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
    /// `-bench-workload W1|W2|W3|W4`, read through NSUserDefaults'
    /// NSArgumentDomain. Missing or unrecognized values trap — a wrong
    /// page must fail, never silently measure W1.
    static let workload: String = {
        let raw = UserDefaults.standard.string(forKey: "bench-workload")
        guard let raw, ["W1", "W2", "W3", "W4"].contains(raw) else {
            fatalError(
                "missing or unrecognized -bench-workload launch argument "
                    + "(got \(raw ?? "nil")); expected W1|W2|W3|W4")
        }
        BenchNotify.postReady(raw)
        return raw
    }()
    /// `-bench-drive swipe|auto` (default swipe); `auto` programs start
    /// only when the runner posts the `dev.bench.begin` Darwin
    /// notification inside its measure block — never at appear.
    static let autoDrive: Bool = {
        let raw = UserDefaults.standard.string(forKey: "bench-drive") ?? "swipe"
        guard raw == "swipe" || raw == "auto" else {
            fatalError("unrecognized -bench-drive value \(raw); expected swipe|auto")
        }
        return raw == "auto"
    }()
}

/// Darwin-notification handshake for the `auto` drive. The runner posts
/// `dev.bench.begin` inside its `measure` block; the app posts
/// `dev.bench.done` when the fling program finishes. AX queries cannot
/// carry the signal on the workloads that need `auto` — the content
/// window has no swipeable hit target — and Darwin notifications are the
/// same mechanism every contestant uses on every platform.
enum BenchNotify {
    private static var token: Int32 = 0
    private static var armed = false

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

    /// Runs `block` on the main queue on each `dev.bench.begin` post.
    /// notify_register_dispatch delivers the post as a real mach wake —
    /// a check-token `notify_check` poll on the main queue can starve under
    /// XCUITest's launch context while `notify_post` still works.
    static func onBegin(_ block: @escaping () -> Void) {
        guard !armed else { return }
        armed = true
        let st = notify_register_dispatch("dev.bench.begin", &token,
                                          DispatchQueue.main) { _ in
            dbg("begin heard")
            notify_post("dev.bench.ack")  // stops the runner's reposts
            // Every observed post is a drive request — measure() invokes
            // its block once per iteration plus a warm-up, each needing a
            // real program. Overlap is prevented by the `driving` guard
            // in drive(), and the runner stops posting after the ack, so
            // a backlog is bounded to one in-flight repost.
            block()
        }
        dbg("onBegin armed status=\(st)")
        guard st == UInt32(NOTIFY_STATUS_OK) else {
            // Dispatch registration failing is not survivable: no mach wake
            // means no program ever starts — surface it instead of stalling.
            dbg("onBegin dispatch registration FAILED status=\(st)")
            return
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

    /// Dispatch tokens carry no latched flag to consume — a begin that
    /// raced the ack arrives as an ordinary post and is filtered by the
    /// `driving` guard in `drive()`. Kept for call-site symmetry.
    static func discardLatchedBegin() {}
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!

    func applicationWillFinishLaunching(_ notification: Notification) {
        BenchNotify.dbg("willFinishLaunching")
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        BenchNotify.dbg("didFinishLaunching start")
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 640),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered, defer: false)
        window.title = "Bench AppKit"
        switch Bench.workload {
        case "W2": window.contentViewController = FeedViewController()
        case "W3": window.contentViewController = MotionViewController()
        case "W4": window.contentViewController = TextBenchViewController()
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
        DispatchQueue.main.async { print("BENCH_READY") }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
}

// MARK: - W1

final class HelloViewController: NSViewController {
    private var count = 0
    private let label = NSTextField(labelWithString: "Count: 0")

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 960, height: 640))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        label.font = .systemFont(ofSize: NSFont.systemFontSize * 2)
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
    private var driving = false

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 960, height: 640))
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
        if Bench.autoDrive {
            BenchNotify.onBegin { [weak self] in self?.drive() }
        }
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

    /// Same fling program as every contestant: 8 bursts down, 2 back up.
    private func drive() {
        // A stray begin backlog must not stack a second program on top.
        guard !driving else { return }
        driving = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [self] in
            tableView.layoutSubtreeIfNeeded()
            let clip = scrollView.contentView
            let docH = tableView.bounds.height
            let maxY = max(docH - clip.bounds.height, 0)
            let step = maxY / 8
            var delay: TimeInterval = 0
            for i in 1...8 {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                    NSAnimationContext.runAnimationGroup { ctx in
                        ctx.duration = 0.9
                        ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
                        clip.animator().setBoundsOrigin(
                            NSPoint(x: 0, y: min(CGFloat(i) * step, maxY)))
                    }
                }
                delay += 1.15
            }
            for i in stride(from: 8, through: 0, by: -4) {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                    NSAnimationContext.runAnimationGroup { ctx in
                        ctx.duration = 0.9
                        ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
                        clip.animator().setBoundsOrigin(
                            NSPoint(x: 0, y: max(CGFloat(i) * step, 0)))
                    }
                }
                delay += 1.15
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                self.driving = false
                BenchNotify.discardLatchedBegin()
                BenchNotify.postDone()
            }
        }
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
        title.font = .systemFont(ofSize: NSFont.systemFontSize)
        subtitle.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        subtitle.textColor = .secondaryLabelColor
        time.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        time.textColor = .secondaryLabelColor
        let lines = NSStackView(views: [title, subtitle])
        lines.orientation = .vertical
        lines.alignment = .leading
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

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: fieldW, height: fieldH))
        view.wantsLayer = true
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
            v.alphaValue = 0.3 + rng.next() * 0.7
            view.addSubview(v)
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
            // Rotation via layer transform (kept off animator for simplicity).
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
    private let paragraphs: [String] = [
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

    private let scrollView = NSScrollView()
    private var driving = false

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 960, height: 640))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 0
        stack.translatesAutoresizingMaskIntoConstraints = false
        for i in 0..<50 {
            let l = NSTextField(wrappingLabelWithString: paragraphs[i % paragraphs.count])
            l.maximumNumberOfLines = 0
            stack.addArrangedSubview(l)
            l.leadingAnchor.constraint(equalTo: stack.leadingAnchor, constant: 16)
                .isActive = true
            stack.setCustomSpacing(12, after: l)
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
        if Bench.autoDrive {
            BenchNotify.onBegin { [weak self] in self?.drive() }
        }
    }

    private func drive() {
        guard !driving else { return }
        driving = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [self] in
            view.layoutSubtreeIfNeeded()
            let clip = scrollView.contentView
            let docH = scrollView.documentView?.frame.height ?? 0
            // AppKit is flipped via the document; scroll origin counts up.
            let maxY = max(docH - clip.bounds.height, 0)
            let step = maxY / 8
            var delay: TimeInterval = 0
            for i in 1...8 {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                    NSAnimationContext.runAnimationGroup { ctx in
                        ctx.duration = 0.9
                        ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
                        clip.animator().setBoundsOrigin(
                            NSPoint(x: 0, y: max(maxY - CGFloat(i) * step, 0)))
                    }
                }
                delay += 1.15
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                self.driving = false
                BenchNotify.discardLatchedBegin()
                BenchNotify.postDone()
            }
        }
    }
}
