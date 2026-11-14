import SwiftUI
import Foundation

struct MetaProbe: View {
    @Environment(\.displayScale) var displayScale
    var body: some View {
        ZStack {
            // Respects safe area: frame = safe region, reports device insets.
            GeometryReader { g in
                Color.clear.onAppear {
                    Store.shared.record("safeRegion", g.frame(in: .global))
                    let s = g.safeAreaInsets
                    Store.shared.recordMeta("safeInsets", [
                        "top": s.top, "leading": s.leading,
                        "bottom": s.bottom, "trailing": s.trailing])
                }
            }
            // Ignores safe area: frame = full window.
            GeometryReader { g in
                Color.clear.onAppear {
                    Store.shared.record("root", g.frame(in: .global))
                    Store.shared.recordMeta("window", [
                        "w": g.size.width, "h": g.size.height])
                    Store.shared.recordMeta("displayScale", Double(displayScale))
                }
            }
            .ignoresSafeArea()
        }
    }
}

// JSONSerialization rejects non-finite numbers; frames and probe answers can
// legitimately contain ±inf (e.g. an unspecified-axis answer), so map them to
// strings before writing.
func sanitizeJSON(_ v: Any) -> Any {
    switch v {
    case let d as Double:
        if d.isNaN { return "nan" }
        if d == .infinity { return "inf" }
        if d == -.infinity { return "-inf" }
        return d
    case let f as CGFloat:
        return sanitizeJSON(Double(f))
    case let a as [Any]:
        return a.map(sanitizeJSON)
    case let d as [String: Any]:
        return d.mapValues(sanitizeJSON)
    default:
        return v
    }
}

func dumpResults() {
    var snap = Store.shared.snapshot().mapValues(sanitizeJSON)
    snap["platform"] = {
        #if os(iOS)
        return "ios"
        #else
        return "macos"
        #endif
    }()
    snap["osVersion"] = ProcessInfo.processInfo.operatingSystemVersionString
    guard let data = try? JSONSerialization.data(
        withJSONObject: snap, options: [.prettyPrinted, .sortedKeys]) else { return }
    let env = ProcessInfo.processInfo.environment
    let url = env["TWINS_OUT"].map { URL(fileURLWithPath: $0) }
        ?? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("twins-swiftui.json")
    try? data.write(to: url)
    // Also echo to stdout so logs capture it.
    if let s = String(data: data, encoding: .utf8) { print("TWINS_JSON_BEGIN\n\(s)\nTWINS_JSON_END") }
}

@main
struct LayoutTwinsApp: App {
    var body: some Scene {
        WindowGroup {
            ZStack {
                MetaProbe()
                rootCase(ProcessInfo.processInfo.environment["TWINS_CASE"] ?? "all")
            }
            .onAppear {
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.5) {
                    dumpResults()
                    // Headless runs exit once results are written; the driver
                    // captures stdout (simctl launch --console) or the file.
                    if ProcessInfo.processInfo.environment["TWINS_EXIT"] == "1" {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                            exit(0)
                        }
                    }
                }
            }
        }
    }
}
