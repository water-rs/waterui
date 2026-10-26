import SwiftUI
import WidgetKit

/// What the extension hands a `#[widget]` function for each timeline
/// request — serialized to the JSON `WidgetContext` Rust decodes.
public struct WUIWidgetContext {
    public let family: String
    public let displaySize: CGSize
}

/// One `#[widget]` declaration, realized as a `WidgetKit` `Widget`.
///
/// The generated `WidgetBundle` instantiates one per
/// `waterui_meta_widget_*` record; `snapshot` is the
/// `waterui_widget_<crate>_<fn>` export the record pairs with.
public struct WaterUIWidget: Widget {
    public let kind: String
    public let displayName: String
    public let widgetDescription: String
    public let families: [WidgetFamily]
    public let snapshot: WUISnapshotProvider

    public init(
        kind: String,
        displayName: String,
        description: String,
        families: [WidgetFamily],
        snapshot: @escaping WUISnapshotProvider
    ) {
        self.kind = kind
        self.displayName = displayName
        self.widgetDescription = description
        self.families = families
        self.snapshot = snapshot
    }

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: WUITimelineProvider(snapshot: snapshot)) { entry in
            WUINodeView(entry.node)
        }
        .configurationDisplayName(displayName)
        .description(widgetDescription)
        .supportedFamilies(families)
    }

    /// `Widget`'s default-initializer requirement — the generated
    /// `WidgetBundle` always instantiates the parameterized form above.
    public init() {
        self.init(
            kind: "",
            displayName: "",
            description: "",
            families: [],
            snapshot: { _, _, out in
                out.assumingMemoryBound(to: WUISnapshotBuffer.self).pointee = WUISnapshotBuffer()
            }
        )
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
        let decoder = JSONDecoder()
        // Rust `Date` is seconds since the Unix epoch.
        decoder.dateDecodingStrategy = .secondsSince1970
        guard let timeline = try? decoder.decode(WUITimeline.self, from: data) else {
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
