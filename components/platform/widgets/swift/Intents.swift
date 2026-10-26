import Foundation

/// The `waterui_intent_<crate>_<hash>` C export a generated `WUIIntent_<hash>`
/// calls — decodes the payload and runs the declared `fn perform(self)`.
public typealias WUIIntentHandler = (UnsafePointer<UInt8>, Int) -> Void

/// Runs a `waterui_intent_*` export against the intent's serialized
/// payload — UTF-8 bytes passed straight through to Rust.
public func performWidgetIntent(_ handler: WUIIntentHandler, payload: String) {
    var payload = payload
    payload.withUTF8 { raw in
        handler(raw.baseAddress!, raw.count)
    }
}
