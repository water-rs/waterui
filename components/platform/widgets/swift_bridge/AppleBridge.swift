import Foundation
import WidgetKit

/// Requests `WidgetKit` to reload timelines for a single widget `kind`.
@_cdecl("waterui_widgets_apple_reload")
public func waterui_widgets_apple_reload(
    _ kindPtr: UnsafePointer<UInt8>?,
    _ kindLen: Int
) {
    guard let kindPtr, kindLen > 0 else {
        WidgetCenter.shared.reloadAllTimelines()
        return
    }
    let bytes = UnsafeBufferPointer(start: kindPtr, count: kindLen)
    let kind = String(decoding: bytes, as: UTF8.self)
    WidgetCenter.shared.reloadTimelines(ofKind: kind)
}

/// Requests `WidgetKit` to reload timelines for every widget declared by the app.
@_cdecl("waterui_widgets_apple_reload_all")
public func waterui_widgets_apple_reload_all() {
    WidgetCenter.shared.reloadAllTimelines()
}

/// Resolves the shared App Group widget store directory for the running app or
/// widget extension from the `WaterUIWidgetAppGroup` key in `Info.plist`.
///
/// Invokes `callback` with the UTF-8 path bytes when available, or `(nil, 0)`
/// when the bundle has no configured App Group container.
@_cdecl("waterui_widgets_apple_shared_container_path")
public func waterui_widgets_apple_shared_container_path(
    _ callback: @convention(c) (UnsafePointer<UInt8>?, Int, UnsafeMutableRawPointer?) -> Void,
    _ context: UnsafeMutableRawPointer?
) {
    guard
        let groupId = Bundle.main.object(forInfoDictionaryKey: "WaterUIWidgetAppGroup") as? String,
        !groupId.isEmpty,
        let containerUrl = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: groupId
        )
    else {
        callback(nil, 0, context)
        return
    }
    let storeUrl = containerUrl.appendingPathComponent("waterui-widgets", isDirectory: true)
    var path = storeUrl.path
    path.withUTF8 { bytes in
        callback(bytes.baseAddress, bytes.count, context)
    }
}
