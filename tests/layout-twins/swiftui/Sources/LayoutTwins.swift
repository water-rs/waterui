import SwiftUI
import Foundation

// Measurement store: part id -> frame in global (window) coordinates.
// All writers are view-lifecycle callbacks (layout runs on the main thread);
// the lock keeps the compiler happy without actor isolation.
final class Store: @unchecked Sendable {
    static let shared = Store()
    private let lock = NSLock()
    private var frames: [String: [Double]] = [:]
    private var probes: [String: [[String: Any]]] = [:]
    private var meta: [String: Any] = [:]

    func record(_ name: String, _ f: CGRect) {
        lock.lock(); defer { lock.unlock() }
        frames[name] = [f.origin.x, f.origin.y, f.width, f.height]
    }
    func recordProbe(_ name: String, proposal: ProposedViewSize, answer: CGSize) {
        lock.lock(); defer { lock.unlock() }
        func d(_ v: CGFloat?) -> Any {
            guard let v = v else { return "nil" }
            if v == .infinity { return "inf" }
            return v
        }
        var arr = probes[name] ?? []
        arr.append([
            "pw": d(proposal.width), "ph": d(proposal.height),
            "aw": answer.width, "ah": answer.height,
        ])
        probes[name] = arr
    }
    func recordMeta(_ key: String, _ value: Any) {
        lock.lock(); defer { lock.unlock() }
        meta[key] = value
    }
    func snapshot() -> [String: Any] {
        lock.lock(); defer { lock.unlock() }
        return ["frames": frames, "probes": probes, "meta": meta]
    }
}

// Tag a view so its global-coordinate frame is recorded under `name`.
struct TagMod: ViewModifier {
    let name: String
    func body(content: Content) -> some View {
        content
            .onGeometryChange(for: CGRect.self,
                              of: { $0.frame(in: .global) }) { f in
                Store.shared.record(name, f)
            }
    }
}
extension View {
    func tag(_ name: String) -> some View { modifier(TagMod(name: name)) }
}

// Custom Layout wrapper that probes its single child with explicit
// ProposedViewSize values (nil / infinity / zero / finite) so the child's
// answers are recorded, then behaves as a pass-through.
struct ProbeLayout: Layout {
    let name: String
    let proposals: [ProposedViewSize]

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews,
                      cache: inout ()) -> CGSize {
        for p in proposals {
            let a = subviews[0].sizeThatFits(p)
            Store.shared.recordProbe(name, proposal: p, answer: a)
        }
        return subviews[0].sizeThatFits(proposal)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize,
                       subviews: Subviews, cache: inout ()) {
        subviews[0].place(at: bounds.origin, proposal: proposal)
    }
}

// Rigid leaf answering (w,h) on both axes.
@MainActor func R(_ w: CGFloat, _ h: CGFloat, _ tag: String) -> some View {
    Rectangle().fill(Color.orange).frame(width: w, height: h).tag(tag)
}
// Flexible fill (stretches on both axes).
@MainActor func Fill(_ tag: String) -> some View {
    Rectangle().fill(Color.blue).tag(tag)
}
// Empty view (not a stack member).
@MainActor func E(_ tag: String) -> some View {
    EmptyView().tag(tag)
}
// The A2 text: 20 breakable monospaced glyphs ~10pt each, line box ~20.
@MainActor func twinsText() -> Text {
    Text("mmmmmmmmmm mmmmmmmmmm")
        .font(.system(size: 16.7, design: .monospaced))
}
@MainActor func T(_ tag: String) -> some View { twinsText().tag(tag) }
