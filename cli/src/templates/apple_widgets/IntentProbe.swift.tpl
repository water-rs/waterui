import AppIntents
import Foundation

@_extern(c, "{{ provider_symbol }}")
private func recordProvider(
    _ ptr: UnsafePointer<UInt8>,
    _ len: Int,
    _ out: UnsafeMutableRawPointer
)

@_extern(c, "{{ steps_symbol }}")
private func stepsProvider(
    _ ptr: UnsafePointer<UInt8>,
    _ len: Int,
    _ out: UnsafeMutableRawPointer
)

private struct RecordPayload: Decodable {
    let outputPath: String
    let value: String
    let nextSteps: UInt32
}

@main
struct IntentProbe {
    static func main() async throws {
        let storePath = FileManager.default.currentDirectoryPath + "/shared-store"
        let context = WUIWidgetContext(
            family: "systemSmall",
            displaySize: CGSize(width: 170, height: 44),
            storePath: storePath
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
        try await perform({{ intent_type }}(payload: node.payload, storePath: storePath))
        let actual = try Data(contentsOf: URL(fileURLWithPath: payload.outputPath))
        precondition(actual == Data(payload.value.utf8), "the Rust handler must write its payload")

        // Verify the intent mutated the shared widget store and the next
        // widget timeline reads the updated value.
        let steps = try WUITimeline.decode(from: runSnapshot(stepsProvider, context: context))
        precondition({
            if case .stack(let stack) = steps.entries[0].content {
                return stack.children.contains { child in
                    if case .text(let text) = child {
                        return text.content.contains("steps=\(payload.nextSteps)")
                    }
                    return false
                }
            }
            return false
        }(), "steps widget must read the updated step count written by the intent")
    }

    private static func perform<I: AppIntent>(_ intent: I) async throws {
        precondition(I.description != nil, "the generated description must witness AppIntent")
        _ = try await intent.perform()
    }
}
