import AppIntents
import Foundation

/// The `AppIntent` a declared `Intent` node performs.
///
/// The node's `name` is the declared intent identifier; performing the
/// intent opens the host application with `waterui-intent://<name>`, which
/// the application handles through its `onOpenURL` — the deep-link contract
/// is how an intent reaches the app's `WaterUI` code from an extension that
/// shares no process with it.
public struct WaterUIDeclaredIntent: AppIntent {
    public static let title: LocalizedStringResource = "WaterUI Intent"

    @Parameter(title: "Name")
    public var name: String

    public init() {
        name = ""
    }

    public init(name: String) {
        self.name = name
    }

    public func perform() async throws -> some IntentResult {
        guard
            let encoded = name.addingPercentEncoding(withAllowedCharacters: .urlHostAllowed),
            let url = URL(string: "waterui-intent://\(encoded)")
        else {
            fatalError("declared widget intent name does not form a URL: \(name)")
        }
        return .result(opensIntent: OpenURLIntent(url))
    }
}
