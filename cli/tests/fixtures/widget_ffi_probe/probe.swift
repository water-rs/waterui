import Foundation

@_extern(c, "waterui_widget_widget_app_steps")
func stepsProvider(_ ptr: UnsafePointer<UInt8>, _ len: Int, _ out: UnsafeMutableRawPointer)

@_extern(c, "waterui_widget_widget_app_now")
func nowProvider(_ ptr: UnsafePointer<UInt8>, _ len: Int, _ out: UnsafeMutableRawPointer)

let context = WUIWidgetContext(
    family: "systemSmall",
    displaySize: CGSize(width: 170, height: 44)
)

// Context-aware: the fixture echoes family+size into its stack's text,
// and produces two explicitly dated entries with an After policy.
let stepsJSON = runSnapshot(stepsProvider, context: context)
let steps = try WUITimeline.decode(from: stepsJSON)
precondition(steps.entries.count == 2, "steps produced \(steps.entries.count) entries")
precondition(
    steps.entries[0].date.timeIntervalSince1970 == 1_700_000_000
        && steps.entries[1].date.timeIntervalSince1970 == 1_700_000_060,
    "steps' entry dates are 1700000000 and 1700000060"
)
precondition({
    if case .after(let date) = steps.policy {
        return date.timeIntervalSince1970 == 1_700_000_120
    }
    return false
}(), "steps reloads after 1700000120")
precondition({
    if case .stack(let stack) = steps.entries[0].content {
        return stack.children.contains { child in
            if case .text(let text) = child {
                return text.content.contains("SystemSmall") && text.content.contains("170x44")
            }
            return false
        }
    }
    return false
}(), "steps' stack echoes the context into a text child")

// Context-free: the same decode runs with no abort.
let now = try WUITimeline.decode(from: runSnapshot(nowProvider, context: context))
precondition(!now.entries.isEmpty, "now produced no entries")
