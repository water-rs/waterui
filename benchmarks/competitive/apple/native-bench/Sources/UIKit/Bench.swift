// Competitive benchmark — UIKit contestant, workloads W1–W6.
import UIKit

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

func rowColor(_ i: Int) -> UIColor {
    [
        UIColor(red: 0x3B / 255, green: 0x82 / 255, blue: 0xF6 / 255, alpha: 1),
        UIColor(red: 0x10 / 255, green: 0xB9 / 255, blue: 0x81 / 255, alpha: 1),
        UIColor(red: 0xF5 / 255, green: 0x9E / 255, blue: 0x0B / 255, alpha: 1),
        UIColor(red: 0xEF / 255, green: 0x44 / 255, blue: 0x44 / 255, alpha: 1),
        UIColor(red: 0x8B / 255, green: 0x5C / 255, blue: 0xF6 / 255, alpha: 1),
        UIColor(red: 0xEC / 255, green: 0x48 / 255, blue: 0x99 / 255, alpha: 1),
    ][i % 6]
}

func timestamp(_ i: Int) -> String { String(format: "%02d:%02d", (i / 60) % 24, i % 60) }

enum Bench {
    /// `-bench-workload w1..=w6` — workload ids are exact lowercase
    /// strings; any other value traps, never silently measure w1.
    static let workload: String = {
        let raw = UserDefaults.standard.string(forKey: "bench-workload")
        guard let raw, ["w1", "w2", "w3", "w4", "w5", "w6"].contains(raw) else {
            fatalError(
                "missing or unrecognized -bench-workload launch argument "
                    + "(got \(raw ?? "nil")); expected w1..=w6")
        }
        BenchNotify.postReady(raw)
        return raw
    }()

    /// `-bench-step N` — one launch renders one capacity step; a
    /// missing, malformed or off-ladder step traps (w5/w6 only).
    static let capacityStep: Int? = {
        let raw = UserDefaults.standard.integer(forKey: "bench-step")
        return raw == 0 ? nil : raw
    }()

    static func step(_ ladder: [Int]) -> Int {
        guard let step = capacityStep, ladder.contains(step) else {
            fatalError(
                "missing or off-ladder -bench-step (got "
                    + "\(capacityStep.map(String.init) ?? "nil")); "
                    + "expected one of \(ladder)")
        }
        return step
    }
}

/// Darwin-notification readiness handshake. AX queries cannot
/// carry the signal: a workload can stall the app's accessibility
/// server for tens of seconds while it materializes, and a timed-out
/// query fails the test instead of driving it. The app posts
/// `dev.bench.done` from its own workload logic after the declared
/// settle+hold following the first frame — no native timer substitutes.
enum BenchNotify {
    /// Posts `dev.bench.done` — a capacity launch's measurement ends.
    static func postDone() { notify_post("dev.bench.done") }

    /// Posts `dev.bench.ready.<bundle-id>.<w>` once the workload argument
    /// has resolved — the runner waits for this post to confirm the
    /// argument arrived, instead of a deep AX query (the 10k-row feed's
    /// accessibility tree takes minutes to materialize).
    static func postReady(_ workload: String) {
        let bid = Bundle.main.bundleIdentifier ?? "unknown"
        notify_post("dev.bench.ready.\(bid).\(workload)")
    }

    /// Capacity launches post `done` settle 1 s + hold 4 s after the
    /// first frame (viewDidAppear is the first-frame boundary for a
    /// pinned step launch — METHOD, WORKLOADS.md).
    static func armDoneAfterHold() {
        DispatchQueue.main.asyncAfter(deadline: .now() + 5.0) {
            BenchNotify.postDone()
        }
    }
}

final class RootViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        let child: UIViewController
        switch Bench.workload {
        case "w2": child = FeedViewController()
        case "w3": child = MotionViewController()
        case "w4": child = TextBenchViewController()
        case "w5": child = MotionCapacityViewController()
        case "w6": child = FeedCapacityViewController()
        default: child = HelloViewController()
        }
        // The runner asserts this identifier after launch. A plain container
        // view never enters the AX tree, so the id rides on a dedicated
        // minimal label (labels are always accessibility elements).
        let marker = UILabel(frame: CGRect(x: 0, y: 0, width: 1, height: 1))
        marker.font = .systemFont(ofSize: 1)
        marker.text = "bench-workload-\(Bench.workload)"
        marker.accessibilityIdentifier = "bench-workload-\(Bench.workload)"
        view.addSubview(marker)
        addChild(child)
        child.view.frame = view.bounds
        child.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(child.view)
        child.didMove(toParent: self)
    }
}

// MARK: - W1

final class HelloViewController: UIViewController {
    private var count = 0
    private let label = UILabel()

    override func viewDidLoad() {
        super.viewDidLoad()
        label.text = "Count: 0"
        label.font = .systemFont(ofSize: 20)
        let button = UIButton(type: .system, primaryAction: UIAction { [weak self] _ in
            guard let self else { return }
            self.count += 1
            self.label.text = "Count: \(self.count)"
        })
        button.setTitle("Increment", for: .normal)
        button.accessibilityIdentifier = "increment-button"
        let stack = UIStackView(arrangedSubviews: [label, button])
        stack.axis = .vertical
        stack.spacing = 16
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }
}

// MARK: - W2

final class FeedCell: UITableViewCell {
    static let reuseID = "feed"
    private let avatar = UIView()
    private let title = UILabel()
    private let subtitle = UILabel()
    private let time = UILabel()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        avatar.layer.cornerRadius = 20
        avatar.clipsToBounds = true
        title.font = .systemFont(ofSize: 16)
        subtitle.font = .systemFont(ofSize: 13)
        subtitle.textColor = .secondaryLabel
        time.font = .systemFont(ofSize: 13)
        time.textColor = .secondaryLabel
        let lines = UIStackView(arrangedSubviews: [title, subtitle])
        lines.axis = .vertical
        lines.alignment = .leading
        lines.spacing = 4
        let row = UIStackView(arrangedSubviews: [avatar, lines, time])
        row.spacing = 12
        row.alignment = .center
        row.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            avatar.widthAnchor.constraint(equalToConstant: 40),
            avatar.heightAnchor.constraint(equalToConstant: 40),
        ])
        contentView.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            row.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            row.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10),
            row.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -10),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(_ i: Int) {
        avatar.backgroundColor = rowColor(i)
        title.text = "Row title \(i)"
        subtitle.text = "Second line of subtitle for item \(i)"
        time.text = timestamp(i)
    }
}

final class FeedViewController: UITableViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.register(FeedCell.self, forCellReuseIdentifier: FeedCell.reuseID)
    }
    override func tableView(_ tv: UITableView, numberOfRowsInSection section: Int) -> Int {
        10_000
    }
    override func tableView(_ tv: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tv.dequeueReusableCell(
            withIdentifier: FeedCell.reuseID, for: indexPath) as! FeedCell
        cell.configure(indexPath.row)
        return cell
    }
}

// MARK: - W3

final class MotionViewController: UIViewController {
    private let fieldW: CGFloat = 720
    private let fieldH: CGFloat = 440

    private lazy var field: UIView = {
        let f = UIView(frame: CGRect(x: 0, y: 0, width: fieldW, height: fieldH))
        f.clipsToBounds = true
        f.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(f)
        NSLayoutConstraint.activate([
            f.widthAnchor.constraint(equalToConstant: fieldW),
            f.heightAnchor.constraint(equalToConstant: fieldH),
            f.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            f.topAnchor.constraint(
                equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
        ])
        return f
    }()

    override func viewDidLoad() {
        super.viewDidLoad()
        for i in 0..<200 {
            var rng = XorShift(seed: 0xD1B5_4A32_D192_ED03 ^ UInt64(i) &* 0x2545_F491_4F6C_DD1D)
            let v = UIView(
                frame: CGRect(
                    x: rng.next() * (fieldW - 40), y: rng.next() * (fieldH - 40),
                    width: 40, height: 40))
            v.backgroundColor = rowColor(i)
            v.layer.cornerRadius = 10
            v.transform = CGAffineTransform(rotationAngle: rng.next() * .pi * 2)
            v.alpha = 0.3 + rng.next() * 0.7
            field.addSubview(v)
            animate(v, index: i)
        }
    }

    private func animate(_ v: UIView, index: Int) {
        let duration = 1.2 + Double(index % 5) * 0.2
        var rng = XorShift(seed: 0x9E37_79B9_7F4A_7C15 ^ UInt64(index) &* 0xBF58_476D_1CE4_E5B9)
        func step() {
            UIView.animate(
                withDuration: duration, delay: 0, options: [.curveEaseInOut, .allowUserInteraction]
            ) {
                v.center = CGPoint(
                    x: rng.next() * (self.fieldW - 40) + 20,
                    y: rng.next() * (self.fieldH - 40) + 20)
                v.transform = CGAffineTransform(rotationAngle: rng.next() * .pi * 2)
                v.alpha = 0.3 + rng.next() * 0.7
            } completion: { _ in step() }
        }
        step()
    }
}

// MARK: - W4

final class TextBenchViewController: UIViewController {
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

    override func viewDidLoad() {
        super.viewDidLoad()
        let sv = UIScrollView()
        let stack = UIStackView()
        // All 50 paragraphs laid out eagerly — layout cost is part of the
        // measurement, so nothing may be lazy.
        stack.axis = .vertical
        stack.alignment = .leading
        stack.spacing = 6
        for i in 0..<50 {
            let l = UILabel()
            l.numberOfLines = 0
            l.text = paragraphs[i % paragraphs.count]
            l.font = .systemFont(ofSize: 16)
            l.translatesAutoresizingMaskIntoConstraints = false
            let wrap = UIView()
            wrap.addSubview(l)
            NSLayoutConstraint.activate([
                l.leadingAnchor.constraint(equalTo: wrap.leadingAnchor, constant: 16),
                l.trailingAnchor.constraint(equalTo: wrap.trailingAnchor, constant: -16),
                l.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 10),
                l.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -10),
            ])
            stack.addArrangedSubview(wrap)
        }
        sv.addSubview(stack)
        stack.translatesAutoresizingMaskIntoConstraints = false
        sv.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(sv)
        NSLayoutConstraint.activate([
            sv.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            sv.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            sv.topAnchor.constraint(equalTo: view.topAnchor),
            sv.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: sv.contentLayoutGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: sv.contentLayoutGuide.trailingAnchor),
            stack.topAnchor.constraint(equalTo: sv.contentLayoutGuide.topAnchor),
            stack.bottomAnchor.constraint(equalTo: sv.contentLayoutGuide.bottomAnchor),
            stack.widthAnchor.constraint(equalTo: sv.frameLayoutGuide.widthAnchor),
        ])
    }

}

// MARK: - W5 Motion capacity

/// W3's scene at one pinned ladder step (200…25600) — one launch renders
/// one step; `done` is posted from workload logic after the declared
/// settle+hold following the first frame.
final class MotionCapacityViewController: UIViewController {
    private let fieldW: CGFloat = 720
    private let fieldH: CGFloat = 440

    private lazy var field: UIView = {
        let f = UIView(frame: CGRect(x: 0, y: 0, width: fieldW, height: fieldH))
        f.clipsToBounds = true
        f.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(f)
        NSLayoutConstraint.activate([
            f.widthAnchor.constraint(equalToConstant: fieldW),
            f.heightAnchor.constraint(equalToConstant: fieldH),
            f.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            f.topAnchor.constraint(
                equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
        ])
        return f
    }()

    private let steps = [200, 400, 800, 1600, 3200, 6400, 12800, 25600]
    private var rects: [UIView] = []

    override func viewDidLoad() {
        super.viewDidLoad()
        setCount(Bench.step(steps))
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        BenchNotify.armDoneAfterHold()
    }

    private func setCount(_ n: Int) {
        while rects.count > n {
            let v = rects.removeLast()
            v.layer.removeAllAnimations()
            v.removeFromSuperview()
        }
        while rects.count < n {
            let i = rects.count
            var rng = XorShift(
                seed: 0xD1B5_4A32_D192_ED03 ^ UInt64(i) &* 0x2545_F491_4F6C_DD1D)
            let v = UIView(frame: CGRect(
                x: rng.next() * (fieldW - 40), y: rng.next() * (fieldH - 40),
                width: 40, height: 40))
            v.backgroundColor = rowColor(i)
            v.layer.cornerRadius = 10
            v.transform = CGAffineTransform(rotationAngle: rng.next() * .pi * 2)
            v.alpha = 0.3 + rng.next() * 0.7
            field.addSubview(v)
            rects.append(v)
            animate(v, index: i)
        }
        count = n
    }

    private func animate(_ v: UIView, index: Int) {
        let duration = 1.2 + Double(index % 5) * 0.2
        var rng = XorShift(
            seed: 0x9E37_79B9_7F4A_7C15 ^ UInt64(index) &* 0xBF58_476D_1CE4_E5B9)
        func step() {
            UIView.animate(
                withDuration: duration, delay: 0,
                options: [.curveEaseInOut, .allowUserInteraction]
            ) {
                v.center = CGPoint(
                    x: rng.next() * (self.fieldW - 40) + 20,
                    y: rng.next() * (self.fieldH - 40) + 20)
                v.transform = CGAffineTransform(rotationAngle: rng.next() * .pi * 2)
                v.alpha = 0.3 + rng.next() * 0.7
            } completion: { [weak v] _ in
                guard v != nil else { return }
                step()
            }
        }
        step()
    }

}

// MARK: - W6 Feed capacity

/// W2 row with `complexity` extra nested text+shape children.
final class FeedCapacityCell: UITableViewCell {
    static let reuseID = "feedcap"
    private let avatar = UIView()
    private let title = UILabel()
    private let subtitle = UILabel()
    private let time = UILabel()
    private let extras = UIStackView()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        avatar.layer.cornerRadius = 20
        avatar.clipsToBounds = true
        title.font = .systemFont(ofSize: 16)
        subtitle.font = .systemFont(ofSize: 13)
        subtitle.textColor = .secondaryLabel
        time.font = .systemFont(ofSize: 13)
        time.textColor = .secondaryLabel
        extras.axis = .horizontal
        extras.spacing = 4
        let lines = UIStackView(arrangedSubviews: [title, subtitle])
        lines.axis = .vertical
        lines.alignment = .leading
        lines.spacing = 4
        let row = UIStackView(arrangedSubviews: [avatar, lines, extras, time])
        row.spacing = 12
        row.alignment = .center
        row.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            avatar.widthAnchor.constraint(equalToConstant: 40),
            avatar.heightAnchor.constraint(equalToConstant: 40),
        ])
        contentView.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(
                equalTo: contentView.leadingAnchor, constant: 16),
            row.trailingAnchor.constraint(
                equalTo: contentView.trailingAnchor, constant: -16),
            row.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10),
            row.bottomAnchor.constraint(
                equalTo: contentView.bottomAnchor, constant: -10),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    private var cells: [UIView] = []

    func configure(_ i: Int, complexity: Int) {
        avatar.backgroundColor = rowColor(i)
        title.text = "Row title \(i)"
        subtitle.text = "Second line of subtitle for item \(i)"
        time.text = timestamp(i)
        // Cells are reused on bind — the count is fixed for the launch,
        // so rebinds repaint each cell rather than rebuild it.
        while cells.count < complexity {
            let shape = UIView(
                frame: CGRect(x: 0, y: 0, width: 14, height: 14))
            shape.layer.cornerRadius = 4
            shape.widthAnchor.constraint(equalToConstant: 14).isActive = true
            shape.heightAnchor.constraint(equalToConstant: 14).isActive = true
            let cap = UILabel()
            cap.font = .systemFont(ofSize: 12)
            let pair = UIStackView(arrangedSubviews: [shape, cap])
            pair.axis = .vertical
            pair.alignment = .center
            cells.append(pair)
            extras.addArrangedSubview(pair)
        }
        while cells.count > complexity {
            extras.removeArrangedSubview(cells.removeLast())
        }
        for (j, pair) in cells.enumerated() {
            (pair.arrangedSubviews[0]).backgroundColor = rowColor(i + j)
            (pair.arrangedSubviews[1] as! UILabel).text = "c\(j)"
        }
    }
}

/// W2's rows at one pinned complexity (1…64) — one launch renders one
/// step; `done` is posted from workload logic after the declared
/// settle+hold following the first frame.
final class FeedCapacityViewController: UITableViewController {
    private let steps = [1, 2, 4, 8, 16, 32, 64]
    private var complexity = 1

    override func viewDidLoad() {
        super.viewDidLoad()
        complexity = Bench.step(steps)
        tableView.register(
            FeedCapacityCell.self,
            forCellReuseIdentifier: FeedCapacityCell.reuseID)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        BenchNotify.armDoneAfterHold()
    }

    override func tableView(_ tv: UITableView, numberOfRowsInSection section: Int)
        -> Int { 10_000 }

    override func tableView(_ tv: UITableView, cellForRowAt indexPath: IndexPath)
        -> UITableViewCell {
        let cell = tv.dequeueReusableCell(
            withIdentifier: FeedCapacityCell.reuseID,
            for: indexPath) as! FeedCapacityCell
        cell.configure(indexPath.row, complexity: complexity)
        return cell
    }

}
