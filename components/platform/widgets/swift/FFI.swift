import CoreGraphics
import Foundation

/// What the extension hands a `#[widget]` function for each timeline
/// request — serialized to the JSON `WidgetContext` Rust decodes. The
/// type lives with the FFI so it compiles without `WidgetKit`.
public struct WUIWidgetContext {
    public let family: String
    public let displaySize: CGSize
}

/// Mirrors `waterui_widgets::ffi::Buffer` — the serialized timeline bytes a
/// `waterui_widget_*` export hands the extension. The struct only ever
/// travels through pointers: `@_extern` admits no `Swift` structs by value.
public struct WUISnapshotBuffer {
    public var ptr: UnsafePointer<UInt8>?
    public var len: Int
    public var cap: Int

    public init(ptr: UnsafePointer<UInt8>? = nil, len: Int = 0, cap: Int = 0) {
        self.ptr = ptr
        self.len = len
        self.cap = cap
    }
}

/// Frees the allocation a `Buffer` owns — the extension hands `ptr`/`len`/
/// `cap` back exactly as it received them.
@_extern(c, "waterui_widgets_snapshot_free")
private func waterui_widgets_snapshot_free(
    _ ptr: UnsafePointer<UInt8>?,
    _ len: Int,
    _ cap: Int
)

/// A `waterui_widget_<crate>_<fn>` export: decodes the context JSON at
/// `ptr`/`len` and writes its timeline `Buffer` through `out` — a raw
/// pointer, since a typed `Swift`-struct pointer is not C-representable
/// either; the `WUISnapshotBuffer` layout mirror is the contract.
public typealias WUISnapshotProvider = (
    UnsafePointer<UInt8>,
    Int,
    UnsafeMutableRawPointer
) -> Void

/// Runs a `waterui_widget_*` export against the `WidgetKit` context and
/// returns the produced `Timeline` JSON — owned copy, the `Rust`
/// allocation freed.
public func runSnapshot(_ snapshot: WUISnapshotProvider, context: WUIWidgetContext) -> Data {
    let contextJSON = contextJSON(context: context)
    var out = WUISnapshotBuffer()
    contextJSON.withUnsafeBytes { raw in
        withUnsafeMutableBytes(of: &out) { outRaw in
            snapshot(
                raw.baseAddress!.assumingMemoryBound(to: UInt8.self),
                raw.count,
                outRaw.baseAddress!
            )
        }
    }
    guard let ptr = out.ptr, out.len > 0 else {
        fatalError("widget produced no timeline bytes")
    }
    defer { waterui_widgets_snapshot_free(ptr, out.len, out.cap) }
    return Data(bytes: ptr, count: out.len)
}

private func contextJSON(context: WUIWidgetContext) -> Data {
    struct SizeJSON: Encodable {
        let width: Double
        let height: Double
    }
    struct ContextJSON: Encodable {
        let family: String
        let displaySize: SizeJSON
    }
    do {
        return try JSONEncoder().encode(ContextJSON(
            family: context.family,
            displaySize: SizeJSON(
                width: context.displaySize.width,
                height: context.displaySize.height
            )
        ))
    } catch {
        fatalError("widget context does not encode: \(error)")
    }
}
