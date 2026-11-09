import SwiftUI
import WidgetKit

/// A `#[widget]` declaration's static contract — the generated
/// `WidgetBundle` emits one conformer per `waterui_meta_widget_*` record,
/// so every `WaterUIWidget<D>` carries real metadata rather than a
/// default-initializer stub.
public protocol WUIWidgetDeclaration {
    static var kind: String { get }
    static var displayName: String { get }
    static var widgetDescription: String { get }
    static var families: [WidgetFamily] { get }
    static var snapshotProvider: WUISnapshotProvider { get }
}

/// One `#[widget]` declaration, realized as a `WidgetKit` `Widget` —
/// `Widget`'s default-initializer requirement is real here because the
/// declaration is the type parameter, not an instance payload.
public struct WaterUIWidget<Declaration: WUIWidgetDeclaration>: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: Declaration.kind,
            provider: WUITimelineProvider(snapshot: Declaration.snapshotProvider)
        ) { entry in
            WUINodeView(entry.node)
                .containerBackground(for: .widget) { Color.clear }
        }
        .configurationDisplayName(Declaration.displayName)
        .description(Declaration.widgetDescription)
        .supportedFamilies(Declaration.families)
    }
}

/// One rendered timeline entry: the snapshot's date and its root node.
public struct WUIEntryValue: TimelineEntry {
    public let date: Date
    public let node: WUINode
}

/// A `TimelineProvider` backed by a `waterui_widget_*` export.
///
/// Every request encodes the `WidgetKit` context as the `WidgetContext`
/// JSON the Rust side decodes, calls the export, and decodes its serialized
/// `Timeline` — the entries carry their dates and the policy is reported
/// verbatim. `placeholder`, `getSnapshot` and `getTimeline` all render the
/// produced entries; there is no stand-in content.
public struct WUITimelineProvider: TimelineProvider {
    public let snapshot: WUISnapshotProvider

    public func placeholder(in context: Context) -> WUIEntryValue {
        producedEntry(in: context)
    }

    public func getSnapshot(in context: Context, completion: @escaping (WUIEntryValue) -> Void) {
        completion(producedEntry(in: context))
    }

    public func getTimeline(in context: Context, completion: @escaping (Timeline<WUIEntryValue>) -> Void) {
        let timeline = producedTimeline(in: context)
        let entries = timeline.entries.map { WUIEntryValue(date: $0.date, node: $0.content) }
        completion(Timeline(entries: entries, policy: reloadPolicy(timeline.policy)))
    }

    /// The first produced entry — `placeholder` and `getSnapshot` render a
    /// real snapshot, never a stand-in.
    private func producedEntry(in context: Context) -> WUIEntryValue {
        let timeline = producedTimeline(in: context)
        guard let first = timeline.entries.first else {
            fatalError("widget produced a timeline with no entries")
        }
        return WUIEntryValue(date: first.date, node: first.content)
    }

    /// Calls the `waterui_widget_*` export and decodes its timeline. A
    /// decode failure is a schema violation the producer emitted — the
    /// provider crashes loudly rather than rendering a fallback.
    private func producedTimeline(in context: Context) -> WUITimeline {
        let widgetContext = WUIWidgetContext(
            family: familyName(context.family),
            displaySize: context.displaySize
        )
        let data = runSnapshot(snapshot, context: widgetContext)
        guard let timeline = try? WUITimeline.decode(from: data) else {
            fatalError("widget timeline JSON does not decode — the producer emitted a schema violation")
        }
        return timeline
    }

    private func reloadPolicy(_ policy: WUIReloadPolicy) -> TimelineReloadPolicy {
        switch policy {
        case .atEnd: return .atEnd
        case .never: return .never
        case let .after(date): return .after(date)
        }
    }

    /// The `WidgetFamily` name serialized into the context JSON — the same
    /// spelling `WidgetContext`'s serde enum expects. `accessory*` families
    /// exist only off macOS.
    private func familyName(_ family: WidgetFamily) -> String {
        switch family {
        case .systemSmall: return "systemSmall"
        case .systemMedium: return "systemMedium"
        case .systemLarge: return "systemLarge"
        case .systemExtraLarge: return "systemExtraLarge"
        #if !os(macOS)
        case .accessoryCircular: return "accessoryCircular"
        case .accessoryRectangular: return "accessoryRectangular"
        case .accessoryInline: return "accessoryInline"
        #endif
        default:
            fatalError("widget family \(family) has no schema representation")
        }
    }
}
