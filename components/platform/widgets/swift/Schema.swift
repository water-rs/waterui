import Foundation

/// The Decodable mirror of `src/schema.rs`. Each node is serialized as
/// `{"type": <variant>, <fields…>}` — the `type` tag selects the decoder and
/// the remaining fields decode the payload struct.
public struct WUITimeline: Decodable {
    public let entries: [WUIEntry]
    public let policy: WUIReloadPolicy
}

public struct WUIEntry: Decodable {
    public let date: Date
    public let content: WUINode
}

public enum WUIReloadPolicy: Decodable {
    case atEnd
    case never
    case after(Date)

    private enum CodingKeys: String, CodingKey { case type, date }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "atEnd": self = .atEnd
        case "never": self = .never
        case "after":
            self = .after(try container.decode(Date.self, forKey: .date))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: container,
                debugDescription: "unknown reload policy \(other)")
        }
    }
}

public indirect enum WUINode: Decodable {
    case text(WUIText)
    case image(WUIImage)
    case stack(WUIStack)
    case shape(WUIShape)
    case color(WUIColor)
    case link(WUILink)
    case intent(WUIIntent)

    private enum CodingKeys: String, CodingKey { case type }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "text": self = .text(try WUIText(from: decoder))
        case "image": self = .image(try WUIImage(from: decoder))
        case "stack": self = .stack(try WUIStack(from: decoder))
        case "shape": self = .shape(try WUIShape(from: decoder))
        case "color": self = .color(try WUIColor(from: decoder))
        case "link": self = .link(try WUILink(from: decoder))
        case "intent": self = .intent(try WUIIntent(from: decoder))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: container,
                debugDescription: "unknown node type \(other)")
        }
    }
}

public struct WUIText: Decodable {
    public let content: String
    public let font: WUIFont?
    public let color: WUIColor?
    public let lineLimit: UInt32?
}

public enum WUIFont: Decodable {
    case named(WUIFontStyle)
    case system(size: Double, weight: WUIFontWeight)

    private enum CodingKeys: String, CodingKey { case kind, style, size, weight }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "named":
            self = .named(try container.decode(WUIFontStyle.self, forKey: .style))
        case "system":
            self = .system(
                size: try container.decode(Double.self, forKey: .size),
                weight: try container.decode(WUIFontWeight.self, forKey: .weight))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: container,
                debugDescription: "unknown font kind \(other)")
        }
    }
}

public enum WUIFontStyle: String, Decodable {
    case largeTitle, title, title2, title3, headline, subheadline
    case body, callout, footnote, caption, caption2
}

public enum WUIFontWeight: String, Decodable {
    case ultraLight, thin, light, regular, medium, semibold
    case bold, heavy, black
}

public enum WUIColor: Decodable {
    case srgb(red: Float, green: Float, blue: Float, opacity: Float)
    case semantic(WUISemanticColor)

    private enum CodingKeys: String, CodingKey {
        case kind, red, green, blue, opacity, semantic
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "srgb":
            self = .srgb(
                red: try container.decode(Float.self, forKey: .red),
                green: try container.decode(Float.self, forKey: .green),
                blue: try container.decode(Float.self, forKey: .blue),
                opacity: try container.decode(Float.self, forKey: .opacity))
        case "semantic":
            self = .semantic(try container.decode(WUISemanticColor.self, forKey: .semantic))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: container,
                debugDescription: "unknown color kind \(other)")
        }
    }
}

public enum WUISemanticColor: String, Decodable {
    case primary, secondary, accent
}

public struct WUIImage: Decodable {
    public let source: WUIImageSource
    public let color: WUIColor?
}

public enum WUIImageSource: Decodable {
    case system(name: String)
    case data(Data)

    private enum CodingKeys: String, CodingKey { case kind, name, data }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "system":
            self = .system(name: try container.decode(String.self, forKey: .name))
        case "data":
            // The Rust side serializes `Vec<u8>` as a JSON byte array.
            let bytes = try container.decode([UInt8].self, forKey: .data)
            self = .data(Data(bytes))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: container,
                debugDescription: "unknown image source kind \(other)")
        }
    }
}

public struct WUIStack: Decodable {
    public let axis: WUIAxis
    public let children: [WUINode]
    public let spacing: Float?
    public let alignment: WUIAlignment?
}

public enum WUIAxis: String, Decodable {
    case horizontal, vertical, depth
}

public struct WUIAlignment: Decodable {
    public let horizontal: WUIHorizontalAlignment?
    public let vertical: WUIVerticalAlignment?
}

public enum WUIHorizontalAlignment: String, Decodable {
    case leading, center, trailing
}

public enum WUIVerticalAlignment: String, Decodable {
    case top, center, bottom
}

public struct WUIShape: Decodable {
    public let kind: WUIShapeKind
    public let fill: WUIColor
}

public enum WUIShapeKind: Decodable {
    case rectangle
    case roundedRectangle(cornerRadius: Float)
    case capsule
    case circle
    case ellipse

    private enum CodingKeys: String, CodingKey { case kind, cornerRadius }
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "rectangle": self = .rectangle
        case "roundedRectangle":
            self = .roundedRectangle(
                cornerRadius: try container.decode(Float.self, forKey: .cornerRadius))
        case "capsule": self = .capsule
        case "circle": self = .circle
        case "ellipse": self = .ellipse
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: container,
                debugDescription: "unknown shape kind \(other)")
        }
    }
}

public struct WUILink: Decodable {
    public let url: String
    public let content: WUINode
}

public struct WUIIntent: Decodable {
    public let name: String
    public let label: WUINode
}
