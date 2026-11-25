import Foundation

/// The `waterui_intent_<crate>_<hash>` C export a generated `WUIIntent_<hash>`
/// calls — decodes the payload and shared store path, then runs the declared
/// `fn perform(self, store: &WidgetStore)`.
public typealias WUIIntentHandler = (
    UnsafePointer<UInt8>,
    Int,
    UnsafePointer<UInt8>,
    Int
) -> Void

/// Runs a `waterui_intent_*` export against the intent's serialized
/// payload and shared widget store path — UTF-8 bytes passed straight through
/// to Rust.
public func performWidgetIntent(
    _ handler: WUIIntentHandler,
    payload: String,
    storePath: String
) {
    var payload = payload
    var storePath = storePath
    payload.withUTF8 { payloadRaw in
        storePath.withUTF8 { storeRaw in
            handler(
                payloadRaw.baseAddress!,
                payloadRaw.count,
                storeRaw.baseAddress!,
                storeRaw.count
            )
        }
    }
}
