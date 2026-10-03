import Dispatch
import Foundation
import SwiftUI
import UIKit

// Reference implementation only. The backend under test remains Rust/objc2.
private struct Metrics: Codable {
  let textFieldHeight: CGFloat
  let rowTop: CGFloat
  let rowLeading: CGFloat
  let rowBottom: CGFloat
  let rowTrailing: CGFloat
  let row24Height: CGFloat
  let row4Height: CGFloat
}

@MainActor
private final class Results {
  var field: CGFloat?
  var margins: NSDirectionalEdgeInsets?
  var row24: CGFloat?
  var row4: CGFloat?

  func startFailureDeadline() {
    // Successful probes finish immediately; this only bounds missing readiness.
    DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [self] in
      var missing: [String] = []
      if field == nil { missing.append("textFieldHeight") }
      if margins == nil {
        missing.append(contentsOf: ["rowTop", "rowLeading", "rowBottom", "rowTrailing"])
      }
      if row24 == nil { missing.append("row24Height") }
      if row4 == nil { missing.append("row4Height") }
      fatalError(
        "Native reference readiness exceeded 30s; missing metrics: \(missing.joined(separator: ", "))"
      )
    }
  }

  func finishWhenReady() {
    guard let field, let margins, let row24, let row4 else { return }
    let metrics = Metrics(
      textFieldHeight: field, rowTop: margins.top,
      rowLeading: margins.leading, rowBottom: margins.bottom,
      rowTrailing: margins.trailing, row24Height: row24, row4Height: row4)
    do {
      let url = URL.documentsDirectory.appendingPathComponent("reference.json")
      let encoder = JSONEncoder()
      encoder.outputFormatting = [.sortedKeys]
      try encoder.encode(metrics).write(to: url, options: .atomic)
      exit(EXIT_SUCCESS)
    } catch { fatalError("Cannot write native reference: \(error)") }
  }
}

@MainActor
private final class RowProbeView: UIView {
  var ready: ((CGFloat) -> Void)?
  override func layoutSubviews() {
    super.layoutSubviews()
    guard window != nil else { return }
    var ancestor = superview
    while let view = ancestor {
      if view is UICollectionViewListCell
        || String(describing: type(of: view)).contains("ListCollectionViewCell")
      {
        guard view.frame.height > 0 else { return }
        ready?(view.frame.height)
        return
      }
      ancestor = view.superview
    }
  }
  override func didMoveToWindow() {
    super.didMoveToWindow()
    setNeedsLayout()
  }
}

private struct RowProbe: UIViewRepresentable {
  let ready: @MainActor (CGFloat) -> Void
  func makeUIView(context: Context) -> RowProbeView {
    let view = RowProbeView()
    view.ready = ready
    return view
  }
  func updateUIView(_ view: RowProbeView, context: Context) { view.ready = ready }
}

@MainActor
private final class MarginsTable: UITableView, UITableViewDataSource {
  var ready: ((NSDirectionalEdgeInsets) -> Void)?
  init() {
    super.init(frame: .zero, style: .insetGrouped)
    dataSource = self
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
  func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { 1 }
  func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
    UITableViewCell(style: .default, reuseIdentifier: nil)
  }
  override func layoutSubviews() {
    super.layoutSubviews()
    if window != nil, let cell = cellForRow(at: IndexPath(row: 0, section: 0)) {
      ready?(cell.contentView.directionalLayoutMargins)
    }
  }
}

@MainActor
private final class ReferenceController: UIViewController {
  private let results = Results()
  private let field = UIHostingController(rootView: TextField("", text: .constant("x")))
  private let table = MarginsTable()

  override func viewDidLoad() {
    super.viewDidLoad()
    results.startFailureDeadline()
    // The original plain-field test measured an unparented hosting controller.
    // Attaching it to this window adds safe-area height to sizeThatFits.
    field.view.frame = CGRect(x: 0, y: 0, width: 402, height: 800)
    results.field =
      field.sizeThatFits(
        in: CGSize(width: 402, height: UIView.layoutFittingCompressedSize.height)
      ).height
    for height in [CGFloat(24), CGFloat(4)] {
      let controller = UIHostingController(
        rootView:
          List {
            Color.red.frame(height: height).background(
              RowProbe { [results] pitch in
                if height == 24 { results.row24 = pitch } else { results.row4 = pitch }
                results.finishWhenReady()
              })
          })
      addChild(controller)
      view.addSubview(controller.view)
      controller.didMove(toParent: self)
    }
    table.ready = { [results] margins in
      results.margins = margins
      results.finishWhenReady()
    }
    view.addSubview(table)
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    // The list probes use the former hosted tests' window proposal.
    let frame = CGRect(x: 0, y: 0, width: 402, height: 874)
    for child in children { child.view.frame = frame }
    table.frame = frame
    results.finishWhenReady()
  }
}

@main
@MainActor
final class ReferenceApp: UIResponder, UIApplicationDelegate {
  var window: UIWindow?
  func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil
  ) -> Bool {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 402, height: 874))
    window.rootViewController = ReferenceController()
    self.window = window
    window.makeKeyAndVisible()
    return true
  }
}
