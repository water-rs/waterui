import Foundation

@_extern(c, "waterui_widget_widget_app_steps")
func stepsProvider(_ ptr: UnsafePointer<UInt8>, _ len: Int, _ out: UnsafeMutableRawPointer)

@_extern(c, "waterui_widget_widget_app_now")
func nowProvider(_ ptr: UnsafePointer<UInt8>, _ len: Int, _ out: UnsafeMutableRawPointer)

let storePath = FileManager.default.currentDirectoryPath + "/widget-store"
let context = WUIWidgetContext(
    family: "systemSmall",
    displaySize: CGSize(width: 170, height: 44),
    storePath: storePath
)

// Context-aware: the fixture echoes family+size+stored steps into its stack's
// text, exercises padding/background/spacer/relative-date/a11y/frame nodes,
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
    guard case .background(let bg) = steps.entries[0].content,
          case .color(.semantic(.accent)) = bg.background,
          case .padding(let padded) = bg.content,
          padded.insets == nil,
          case .stack(let stack) = padded.content
    else {
        return false
    }
    let hasHeadline = stack.children.contains { child in
        if case .text(let text) = child,
           case .localized(let key) = text.content,
           key == "Steps",
           case .named(.headline) = text.font {
            return true
        }
        return false
    }
    let hasSpacer = stack.children.contains { child in
        if case .spacer = child { return true }
        return false
    }
    let hasRelativeDate = stack.children.contains { child in
        if case .text(let text) = child,
           case .date(let date, let style) = text.content,
           date.timeIntervalSince1970 == 1_700_000_060,
           style == .relative {
            return true
        }
        return false
    }
    let hasLabeledIcon = stack.children.contains { child in
        if case .accessibilityLabel(let a11y) = child,
           case .localized(let key) = a11y.label,
           key == "Walking",
           case .image(let img) = a11y.content,
           case .sfSymbol(let name) = img.source,
           name == "figure.walk" {
            return true
        }
        return false
    }
    let hasContextText = stack.children.contains { child in
        if case .text(let text) = child,
           case .verbatim(let raw) = text.content {
            return raw.contains("SystemSmall")
                && raw.contains("170x44")
                && raw.contains("steps=0")
        }
        return false
    }
    let hasFramedShape = stack.children.contains { child in
        if case .frame(let frame) = child,
           frame.width == 48.0,
           frame.height == 8.0,
           case .shape = frame.content {
            return true
        }
        return false
    }
    return hasHeadline && hasSpacer && hasRelativeDate && hasLabeledIcon && hasContextText && hasFramedShape
}(), "steps' stack decodes background, padding, localized headline, spacer, relative date, a11y label, context text, and frame")

// Context-free: the same decode runs with no abort.
let now = try WUITimeline.decode(from: runSnapshot(nowProvider, context: context))
precondition(!now.entries.isEmpty, "now produced no entries")
