import AppIntents
import Foundation

@_extern(c, "{{ provider_symbol }}")
private func recordProvider(
    _ ptr: UnsafePointer<UInt8>,
    _ len: Int,
    _ out: UnsafeMutableRawPointer
)

private struct RecordPayload: Decodable {
    let outputPath: String
    let value: String
}

@main
struct IntentProbe {
    static func main() async throws {
        let context = WUIWidgetContext(
            family: "systemSmall",
            displaySize: CGSize(width: 170, height: 44)
        )
        let timeline = try WUITimeline.decode(from: runSnapshot(recordProvider, context: context))
        precondition(timeline.entries.count == 1)
        guard case .intent(let node) = timeline.entries[0].content else {
            fatalError("the record widget must produce an intent")
        }
        precondition(node.kind == CommandLine.arguments[1])
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let payload = try decoder.decode(RecordPayload.self, from: Data(node.payload.utf8))
        precondition(!FileManager.default.fileExists(atPath: payload.outputPath))
        try await perform({{ intent_type }}(payload: node.payload))
        let actual = try Data(contentsOf: URL(fileURLWithPath: payload.outputPath))
        precondition(actual == Data(payload.value.utf8), "the Rust handler must write its payload")
    }

    private static func perform<I: AppIntent>(_ intent: I) async throws {
        precondition(I.description != nil, "the generated description must witness AppIntent")
        _ = try await intent.perform()
    }
}
