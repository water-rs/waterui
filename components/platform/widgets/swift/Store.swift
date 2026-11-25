import Foundation

/// Resolves the shared App Group container directory provisioned for the host
/// application and its `WidgetKit` extension.
public enum WUIWidgetStore {
    /// `Info.plist` key carrying the shared App Group identifier provisioned by
    /// the `water` CLI on both the application and the `.appex` extension.
    public static let appGroupInfoPlistKey = "WaterUIWidgetAppGroup"

    /// The shared widget store directory inside the App Group container.
    ///
    /// Crashes loudly if `WaterUIWidgetAppGroup` is missing or the system
    /// denies the App Group container — a misconfigured bundle must surface
    /// immediately rather than silently diverging from the host application.
    public static func sharedContainerPath() -> String {
        guard
            let groupId = Bundle.main.object(forInfoDictionaryKey: appGroupInfoPlistKey) as? String,
            !groupId.isEmpty
        else {
            fatalError(
                "widget extension Info.plist has no \(appGroupInfoPlistKey) — cannot locate shared widget store"
            )
        }
        guard
            let containerUrl = FileManager.default.containerURL(
                forSecurityApplicationGroupIdentifier: groupId
            )
        else {
            fatalError(
                "App Group container `\(groupId)` is unavailable — check the com.apple.security.application-groups entitlement on both the app and widget extension"
            )
        }
        return containerUrl
            .appendingPathComponent("waterui-widgets", isDirectory: true)
            .path
    }
}
