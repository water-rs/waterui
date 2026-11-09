import AppKit
import CoreText
import Foundation
import SwiftUI
import WaterUI
import os

final class Model: ObservableObject {
    @Published var changed = false
}

/// Records the environment's own resolved `Font.body` — the same snapshot
/// the mount's host metrics come from, captured inside this view graph.
/// Plain class, not `ObservableObject`: the assignment must not feed back
/// into SwiftUI.
@MainActor
final class FontReference {
    var pointSize: CGFloat?
    var dynamicType: DynamicTypeSize?
}

private struct ResolvedFontReference: View {
    let record: FontReference
    @Environment(\.self) private var environment
    var body: some View {
        record.pointSize =
            CTFontGetSize(Font.body.resolve(in: environment.fontResolutionContext).ctFont)
        record.dynamicType = environment.dynamicTypeSize
        return Color.clear
    }
}

struct HostProbe: View {
    let runtime: WaterUIRuntime
    @ObservedObject var model: Model
    let fontReference: FontReference
    let initialLocale: String
    let changedLocale: String
    var body: some View {
        WaterUIHost(runtime: runtime, resources: .module)
            .background(ResolvedFontReference(record: fontReference))
            .environment(
                \.locale,
                Locale(identifier: model.changed ? changedLocale : initialLocale)
            )
            .environment(\.layoutDirection, model.changed ? .rightToLeft : .leftToRight)
            .environment(\.colorScheme, model.changed ? .dark : .light)
            .environment(\.dynamicTypeSize, model.changed ? .accessibility3 : .large)
    }
}

@MainActor
func firstTextField(in view: NSView) -> NSTextField? {
    if let field = view as? NSTextField { return field }
    for sub in view.subviews {
        if let found = firstTextField(in: sub) { return found }
    }
    return nil
}

/// The scoped mount's own host controller — found through the responder
/// chain, not a class-name string, so the assertion is the real type.
@MainActor
func hostController(of view: NSView) -> WaterUIHostController? {
    var responder: NSResponder? = view.nextResponder
    while let current = responder {
        if let controller = current as? WaterUIHostController {
            return controller
        }
        responder = current.nextResponder
    }
    return nil
}

@main
enum Probe {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        Task { @MainActor in
            do {
                try await run()
                app.terminate(nil)
            } catch {
                fatalError("embedded probe failed: \(error)")
            }
        }
        app.run()
    }

    @MainActor
    static func run() async throws {
        let runtime = await WaterUIRuntime.create()
        let ambientAppearance = NSApp.appearance
        let modelA = Model(), modelB = Model()
        let referenceA = FontReference(), referenceB = FontReference()
        // A updates en-US/LTR/large → ja-JP/RTL/dark/accessibility3.
        // B holds ar-EG/RTL/dark/accessibility3 as constants — its locale
        // never moves off ar-EG, so its mount proves scope independence.
        let hostA = NSHostingView(
            rootView: HostProbe(runtime: runtime, model: modelA, fontReference: referenceA,
                                initialLocale: "en-US", changedLocale: "ja-JP")
        )
        let hostB = NSHostingView(
            rootView: HostProbe(runtime: runtime, model: modelB, fontReference: referenceB,
                                initialLocale: "ar-EG", changedLocale: "ar-EG")
        )
        modelB.changed = true
        hostA.frame = NSRect(x: 0, y: 0, width: 300, height: 200)
        hostB.frame = NSRect(x: 0, y: 0, width: 300, height: 200)
        let windowA = NSWindow(contentRect: .zero, styleMask: [.titled],
                               backing: .buffered, defer: false)
        windowA.contentView = hostA
        let windowB = NSWindow(contentRect: .zero, styleMask: [.titled],
                               backing: .buffered, defer: false)
        windowB.contentView = hostB
        hostA.layoutSubtreeIfNeeded()
        hostB.layoutSubtreeIfNeeded()

        guard let labelA = firstTextField(in: hostA),
              let labelB = firstTextField(in: hostB) else {
            throw CocoaError(.coderReadCorrupt,
                             userInfo: [NSLocalizedDescriptionKey: "no NSTextField mounted"])
        }
        precondition(labelA.stringValue.contains("en-US"),
                     "A starts en-US, got \(labelA.stringValue)")
        precondition(labelB.stringValue.contains("ar-EG"),
                     "B starts ar-EG, got \(labelB.stringValue)")
        guard let expectedA0 = referenceA.pointSize,
              let expectedB0 = referenceB.pointSize else {
            throw CocoaError(.coderReadCorrupt,
                             userInfo: [NSLocalizedDescriptionKey: "reference never resolved"])
        }
        precondition(referenceA.dynamicType == .large, "A starts at .large")
        precondition(referenceB.dynamicType == .accessibility3, "B starts at .accessibility3")
        let fontA0 = (labelA.attributedStringValue
            .attribute(.font, at: 0, effectiveRange: nil) as? NSFont)!.pointSize
        let fontB0 = (labelB.attributedStringValue
            .attribute(.font, at: 0, effectiveRange: nil) as? NSFont)!.pointSize
        // Compare with the SwiftUI-resolved font from the same environment;
        // the platform determines whether the metric changes.
        precondition(abs(fontA0 - expectedA0) < 0.01,
                     "A's label draws at the environment-resolved .body size: \(fontA0) vs \(expectedA0)")
        precondition(abs(fontB0 - expectedB0) < 0.01,
                     "B's label draws at the environment-resolved .body size: \(fontB0) vs \(expectedB0)")
        let fittingA = hostA.fittingSize
        precondition(fittingA.width > 0 && fittingA.height > 0
                     && fittingA.width.isFinite && fittingA.height.isFinite,
                     "A's fitting size is positive and finite")

        modelA.changed = true
        var settled = false
        for _ in 0..<200 {
            try await Task.sleep(nanoseconds: 10_000_000)
            hostA.layoutSubtreeIfNeeded()
            hostB.layoutSubtreeIfNeeded()
            if labelA.stringValue.contains("ja-JP")
                && referenceA.dynamicType == .accessibility3 { settled = true; break }
        }
        precondition(settled, "A's locale update never landed")
        precondition(firstTextField(in: hostA) === labelA,
                     "A's label is the same mounted object after the update")
        precondition(firstTextField(in: hostB) === labelB,
                     "B's label is the same mounted object after the update")
        guard let expectedA1 = referenceA.pointSize else {
            throw CocoaError(.coderReadCorrupt,
                             userInfo: [NSLocalizedDescriptionKey: "A's reference empty after update"])
        }
        let fontA1 = (labelA.attributedStringValue
            .attribute(.font, at: 0, effectiveRange: nil) as? NSFont)!.pointSize
        precondition(abs(fontA1 - expectedA1) < 0.01,
                     "A's label follows the updated environment's resolved size: \(fontA1) vs \(expectedA1)")
        let fontB1 = (labelB.attributedStringValue
            .attribute(.font, at: 0, effectiveRange: nil) as? NSFont)!.pointSize
        precondition(fontB1 == fontB0 && abs(fontB1 - (referenceB.pointSize ?? -1)) < 0.01,
                     "B's font unchanged at its reference size: \(fontB0) → \(fontB1)")

        guard let controller = hostController(of: labelA) else {
            throw CocoaError(.coderReadCorrupt,
                             userInfo: [NSLocalizedDescriptionKey: "no WaterUIHostController found"])
        }
        precondition(controller.view.userInterfaceLayoutDirection == .rightToLeft,
                     "A's host direction is RTL")
        precondition(controller.view.appearance?.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua,
                     "A's host appearance resolves dark")
        precondition(NSApp.appearance === ambientAppearance || NSApp.appearance == ambientAppearance,
                     "the app's ambient appearance is untouched")
        Logger().notice("embedded consumer probe passed")
    }
}
