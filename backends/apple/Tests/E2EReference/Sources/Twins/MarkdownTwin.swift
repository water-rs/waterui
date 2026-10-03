// Twin of examples/markdown: the example renders `include_markdown!("example.md")`
// inside a scroll view. `Text(AttributedString(markdown:))` does not style
// headings, lists, blockquotes or tables, so the twin rebuilds the document as
// the equivalent hand-written SwiftUI tree — what a SwiftUI developer would
// produce for the same content.
//
// WaterUI markdown chrome reproduced here (heading levels consume the
// framework's semantic font slots, resolved to platform text styles):
//   # -> .title bold, ## -> .headline bold, ### -> .body bold
//   fenced blocks get a header row (language name + Copy) over the
//   SurfaceVariant fill, and the code text is the syntect base16-ocean.light
//   palette at Body.size(14).monospaced() (components/foundation/text/src/
//   code.rs `default_rendering`)

import SwiftUI

#if os(iOS)
  import UIKit
#else
  import AppKit
#endif

struct MarkdownTwin: View {
    var body: some View {
        // The example's column: `hstack((button("Find"), spacer()))` above a
        // `zstack` whose top layer is the `when`-hidden search row — the
        // Find control never overlays the document; only the open panel does.
        VStack(spacing: 10) {
            HStack {
                Button("Find") {}
                Spacer()
            }
            .padding(EdgeInsets(top: 14, leading: 14, bottom: 0, trailing: 14))

            ZStack {
                ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text("WaterUI Markdown").font(.title).fontWeight(.bold)
                Text("WaterUI supports rendering **Markdown** content natively across all platforms.")

                heading("Text Formatting")
                Text("You can use **bold**, *italic*, and `inline code` in your text. Combine them for ***bold italic*** text.")

                heading("Code Blocks")
                Text("Here's a Rust example:")
                codeBlock("Rust", [
                    ("fn", .codeKeyword),
                    (" ", .codeText),
                    ("main", .codeFunction),
                    ("() {", .codeText),
                    ("\n    println!(", .codeText),
                    ("\"Hello, WaterUI!\"", .codeString),
                    (");", .codeText),
                    ("\n}", .codeText),
                    ("\n", .codeText),
                ])
                Text("And some Swift code:")
                codeBlock("Swift", [
                    ("import", .codeKeyword),
                    (" SwiftUI", .codeText),
                    ("\n", .codeText),
                    ("\nstruct", .codeKeyword),
                    (" ContentView: View {", .codeText),
                    ("\n    var", .codeKeyword),
                    (" body: some View {", .codeText),
                    ("\n        Text(", .codeText),
                    ("\"Hello, World!\"", .codeString),
                    (")", .codeText),
                    ("\n    }", .codeText),
                    ("\n}", .codeText),
                    ("\n", .codeText),
                ])

                heading("Lists")
                Text("Unordered List").font(.body).fontWeight(.bold)
                VStack(alignment: .leading, spacing: 10) {
                    bullet("First item")
                    bullet("Second item")
                    bullet("Third item")
                }
                Text("Ordered List").font(.body).fontWeight(.bold)
                VStack(alignment: .leading, spacing: 10) {
                    numbered(1, "Step one")
                    numbered(2, "Step two")
                    numbered(3, "Step three")
                }

                heading("Blockquotes")
                HStack(spacing: 0) {
                    Rectangle().fill(Color.secondary.opacity(0.4)).frame(width: 3)
                    Text("WaterUI brings the power of native UI to Rust developers.\nBuild once, run everywhere.")
                        .foregroundStyle(.secondary)
                        .padding(.leading, 10)
                }

                heading("Tables")
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                    GridRow {
                        Text("Platform").fontWeight(.bold)
                        Text("Backend").fontWeight(.bold)
                        Text("Status").fontWeight(.bold)
                    }
                    wuiDivider()
                    GridRow { Text("iOS"); Text("SwiftUI"); Text("Ready") }
                    GridRow { Text("macOS"); Text("AppKit"); Text("Ready") }
                    GridRow { Text("Android"); Text("View"); Text("Ready") }
                }

                wuiDivider()

                Text("Visit [WaterUI on GitHub](https://github.com/water-rs/waterui) for more information.")
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            }

                // `vstack((when(open, search_bar), spacer())).padding()` —
                // the search row contributes nothing while the panel is
                // closed, so the overlay column is just a spacer.
                VStack {
                    Spacer()
                }
                .padding()
            }
        }
    }

    private func heading(_ s: String) -> some View {
        Text(s).font(.headline).fontWeight(.bold)
    }

    // The example emits `hstack((text("• "), item))` / `hstack((text("{}. "),
    // item))` with the default 10pt spacing.
    private func bullet(_ s: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("• ")
            Text(s)
        }
    }

    private func numbered(_ n: Int, _ s: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("\(n). ")
            Text(s)
        }
    }

    // code.rs: VStack(Leading, 8) of the header row and the highlighted source,
    // padding 14 on every edge over the SurfaceVariant fill. The highlight
    // chunks keep the fence's trailing newline, so the rendered text carries a
    // closing empty line.
    private func codeBlock(_ lang: String, _ spans: [(String, CodeSpanColor)]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(lang).fontWeight(.bold).foregroundStyle(.secondary)
                Spacer(minLength: 0)
                Text("Copy").foregroundStyle(.blue)
            }
            Text(spans.reduce(into: AttributedString()) { result, span in
                var part = AttributedString(span.0)
                part.foregroundColor = span.1.color
                result.append(part)
            })
            .font(.system(size: 14, design: .monospaced))
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(14)
        .background(codeSurface)
    }

    /// `Color::new(SurfaceVariantColor)`: the Apple backend resolves the slot
    /// to `tertiarySystemFill` on both platforms
    /// (Sources/WaterUI/WaterUI.swift `makeColorSignalEntries`).
    private var codeSurface: Color {
        #if os(iOS)
            Color(uiColor: .tertiarySystemFill)
        #else
            Color(nsColor: .tertiarySystemFill)
        #endif
    }
}

/// The syntect `base16-ocean.light` scopes `DefaultHighlighter` emits
/// (components/foundation/text/src/highlight.rs).
enum CodeSpanColor {
    /// Theme foreground (#4F5B66): punctuation, plain identifiers, whitespace.
    case codeText
    /// `keyword` / `storage` (#B48EAD).
    case codeKeyword
    /// `entity.name.function` (#8FA1B3).
    case codeFunction
    /// `string` (#A3BE8C).
    case codeString

    var color: Color {
        switch self {
        case .codeText: srgbHex(0x4F5B66)
        case .codeKeyword: srgbHex(0xB48EAD)
        case .codeFunction: srgbHex(0x8FA1B3)
        case .codeString: srgbHex(0xA3BE8C)
        }
    }
}
