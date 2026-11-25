import SwiftUI
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// The restricted `SwiftUI` realization of a widget schema node: every case
/// maps to a native `SwiftUI` view — nothing else can be rendered, because
/// the schema has no other nodes.
public struct WUINodeView: View {
    public let node: WUINode

    public init(_ node: WUINode) {
        self.node = node
    }

    public var body: some View {
        switch node {
        case let .text(text):
            textView(text)
        case let .image(image):
            imageView(image)
        case let .stack(stack):
            stackView(stack)
        case let .shape(shape):
            shapeView(shape)
        case let .color(color):
            colorView(color)
        case let .padding(padding):
            paddingView(padding)
        case let .spacer(spacer):
            spacerView(spacer)
        case let .frame(frame):
            frameView(frame)
        case let .background(background):
            backgroundView(background)
        case let .accessibilityLabel(a11y):
            accessibilityLabelView(a11y)
        case let .link(link):
            linkView(link)
        case let .intent(intent):
            intentView(intent)
        }
    }

    @ViewBuilder
    private func textView(_ text: WUIText) -> some View {
        let base = baseText(text.content)
        let styled = applyBold(base.font(text.font.map(swiftUIFont)), bold: text.bold)
        if let color = text.color {
            if let lineLimit = text.lineLimit {
                styled.foregroundStyle(colorView(color)).lineLimit(Int(lineLimit))
            } else {
                styled.foregroundStyle(colorView(color))
            }
        } else if let lineLimit = text.lineLimit {
            styled.lineLimit(Int(lineLimit))
        } else {
            styled
        }
    }

    private func baseText(_ content: WUITextContent) -> Text {
        switch content {
        case let .localized(key):
            return Text(LocalizedStringKey(key))
        case let .verbatim(text):
            return Text(verbatim: text)
        case let .date(date, style):
            return Text(date, style: swiftUIDateStyle(style))
        }
    }

    private func applyBold(_ text: Text, bold: Bool?) -> Text {
        guard let bold else { return text }
        return text.bold(bold)
    }

    private func swiftUIDateStyle(_ style: WUIDateStyle) -> Text.DateStyle {
        switch style {
        case .relative: return .relative
        case .offset: return .offset
        case .timer: return .timer
        case .date: return .date
        case .time: return .time
        }
    }

    @ViewBuilder
    private func imageView(_ image: WUIImage) -> some View {
        let base: Image = switch image.source {
        case let .sfSymbol(name):
            Image(systemName: name)
        case let .data(data):
            imageFromData(data)
        }
        if let color = image.color {
            base.renderingMode(.template).foregroundStyle(colorView(color))
        } else {
            base
        }
    }

    /// Bytes that do not decode are a schema violation the producer
    /// emitted — fail loudly rather than draw nothing.
    private func imageFromData(_ data: Data) -> Image {
        #if canImport(UIKit)
        guard let image = UIImage(data: data) else {
            fatalError("widget image data does not decode")
        }
        return Image(uiImage: image)
        #elseif canImport(AppKit)
        guard let image = NSImage(data: data) else {
            fatalError("widget image data does not decode")
        }
        return Image(nsImage: image)
        #endif
    }

    @ViewBuilder
    private func stackView(_ stack: WUIStack) -> some View {
        let spacing = stack.spacing.map(CGFloat.init)
        switch stack.axis {
        case .horizontal:
            HStack(alignment: verticalAlignment(stack.alignment?.vertical), spacing: spacing) {
                ForEach(stack.children.indices, id: \.self) { index in
                    WUINodeView(stack.children[index])
                }
            }
        case .vertical:
            VStack(alignment: horizontalAlignment(stack.alignment?.horizontal), spacing: spacing) {
                ForEach(stack.children.indices, id: \.self) { index in
                    WUINodeView(stack.children[index])
                }
            }
        case .depth:
            ZStack(alignment: twoDimensionalAlignment(stack.alignment, default: .center)) {
                ForEach(stack.children.indices, id: \.self) { index in
                    WUINodeView(stack.children[index])
                }
            }
        }
    }

    @ViewBuilder
    private func shapeView(_ shape: WUIShape) -> some View {
        let fill = colorView(shape.fill)
        switch shape.kind {
        case .rectangle:
            Rectangle().fill(fill)
        case let .roundedRectangle(cornerRadius):
            RoundedRectangle(cornerRadius: CGFloat(cornerRadius)).fill(fill)
        case .capsule:
            Capsule().fill(fill)
        case .circle:
            Circle().fill(fill)
        case .ellipse:
            Ellipse().fill(fill)
        }
    }

    /// `WUIColor`'s cases all resolve to a `Color` — concrete, a
    /// `ShapeStyle` for fills and a `View` for the `.color` node alike.
    private func colorView(_ color: WUIColor) -> Color {
        switch color {
        case let .srgb(red, green, blue, opacity):
            Color(.sRGB, red: Double(red), green: Double(green), blue: Double(blue),
                  opacity: Double(opacity))
        case let .semantic(semantic):
            switch semantic {
            case .primary: Color.primary
            case .secondary: Color.secondary
            case .accent: Color.accentColor
            }
        }
    }

    @ViewBuilder
    private func paddingView(_ padding: WUIPadding) -> some View {
        if let insets = padding.insets {
            WUINodeView(padding.content).padding(EdgeInsets(
                top: CGFloat(insets.top),
                leading: CGFloat(insets.leading),
                bottom: CGFloat(insets.bottom),
                trailing: CGFloat(insets.trailing)
            ))
        } else {
            WUINodeView(padding.content).padding()
        }
    }

    @ViewBuilder
    private func spacerView(_ spacer: WUISpacer) -> some View {
        Spacer(minLength: spacer.minLength.map(CGFloat.init))
    }

    @ViewBuilder
    private func frameView(_ frame: WUIFrame) -> some View {
        let alignment = twoDimensionalAlignment(frame.alignment, default: .center)
        let hasFlexBounds = frame.minWidth != nil || frame.maxWidth != nil
            || frame.minHeight != nil || frame.maxHeight != nil
        let hasExactBounds = frame.width != nil || frame.height != nil
        if hasExactBounds && hasFlexBounds {
            WUINodeView(frame.content)
                .frame(
                    width: frame.width.map(CGFloat.init),
                    height: frame.height.map(CGFloat.init),
                    alignment: alignment
                )
                .frame(
                    minWidth: frame.minWidth.map(CGFloat.init),
                    maxWidth: frame.maxWidth.map(cgFloatMaxLength),
                    minHeight: frame.minHeight.map(CGFloat.init),
                    maxHeight: frame.maxHeight.map(cgFloatMaxLength),
                    alignment: alignment
                )
        } else if hasFlexBounds {
            WUINodeView(frame.content).frame(
                minWidth: frame.minWidth.map(CGFloat.init),
                maxWidth: frame.maxWidth.map(cgFloatMaxLength),
                minHeight: frame.minHeight.map(CGFloat.init),
                maxHeight: frame.maxHeight.map(cgFloatMaxLength),
                alignment: alignment
            )
        } else {
            WUINodeView(frame.content).frame(
                width: frame.width.map(CGFloat.init),
                height: frame.height.map(CGFloat.init),
                alignment: alignment
            )
        }
    }

    private func cgFloatMaxLength(_ length: WUIMaxLength) -> CGFloat {
        switch length {
        case let .fixed(value):
            return CGFloat(value)
        case .infinity:
            return .infinity
        }
    }

    @ViewBuilder
    private func backgroundView(_ background: WUIBackground) -> some View {
        WUINodeView(background.content).background {
            WUINodeView(background.background)
        }
    }

    @ViewBuilder
    private func accessibilityLabelView(_ a11y: WUIAccessibilityLabel) -> some View {
        WUINodeView(a11y.content).accessibilityLabel(labelText(a11y.label))
    }

    internal static func labelText(_ label: WUILabelText) -> Text {
        switch label {
        case let .localized(key):
            return Text(LocalizedStringKey(key))
        case let .verbatim(text):
            return Text(verbatim: text)
        }
    }

    private func labelText(_ label: WUILabelText) -> Text {
        Self.labelText(label)
    }

    @ViewBuilder
    private func linkView(_ link: WUILink) -> some View {
        if let url = URL(string: link.url) {
            Link(destination: url) {
                WUINodeView(link.content)
            }
        } else {
            // A URL the producer emitted that cannot be parsed is a schema
            // violation — crash the provider rather than render an
            // untappable label.
            fatalError("widget link URL does not parse: \(link.url)")
        }
    }

    @ViewBuilder
    private func intentView(_ intent: WUIIntent) -> some View {
        WUIDeclaredIntentButton(intent)
    }

    private func swiftUIFont(_ font: WUIFont) -> Font {
        switch font {
        case let .named(style):
            switch style {
            case .largeTitle: return .largeTitle
            case .title: return .title
            case .title2: return .title2
            case .title3: return .title3
            case .headline: return .headline
            case .subheadline: return .subheadline
            case .body: return .body
            case .callout: return .callout
            case .footnote: return .footnote
            case .caption: return .caption
            case .caption2: return .caption2
            }
        case let .system(size, weight):
            return .system(size: size, weight: fontWeight(weight))
        }
    }

    private func fontWeight(_ weight: WUIFontWeight) -> Font.Weight {
        switch weight {
        case .ultraLight: return .ultraLight
        case .thin: return .thin
        case .light: return .light
        case .regular: return .regular
        case .medium: return .medium
        case .semibold: return .semibold
        case .bold: return .bold
        case .heavy: return .heavy
        case .black: return .black
        }
    }

    private func horizontalAlignment(_ alignment: WUIHorizontalAlignment?) -> HorizontalAlignment {
        switch alignment {
        case .leading, nil: return .leading
        case .center: return .center
        case .trailing: return .trailing
        }
    }

    private func verticalAlignment(_ alignment: WUIVerticalAlignment?) -> VerticalAlignment {
        switch alignment {
        case .top, nil: return .top
        case .center: return .center
        case .bottom: return .bottom
        }
    }

    private func twoDimensionalAlignment(
        _ alignment: WUIAlignment?,
        default defaultAlignment: Alignment
    ) -> Alignment {
        guard let alignment else { return defaultAlignment }
        let horizontal = horizontalAlignment(alignment.horizontal)
        let vertical = verticalAlignment(alignment.vertical)
        switch (vertical, horizontal) {
        case (.top, .leading): return .topLeading
        case (.top, .center): return .top
        case (.top, .trailing): return .topTrailing
        case (.center, .leading): return .leading
        case (.center, .center): return .center
        case (.center, .trailing): return .trailing
        case (.bottom, .leading): return .bottomLeading
        case (.bottom, .center): return .bottom
        case (.bottom, .trailing): return .bottomTrailing
        // `VerticalAlignment` is not a closed enum — the schema maps only
        // the nine cases above.
        default: return defaultAlignment
        }
    }
}
