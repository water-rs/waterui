// Material Design Icons as the examples draw them.
//
// `waterui_icons_material_icon` renders an icon as a `Picture` of its SVG
// path filled with the foreground colour at the 24×24 viewBox size (or the
// size the example frames it to). The twin of an example that uses one must
// draw the same path — a "closest SF Symbol" is a different glyph and shows
// up in the parity diff at every icon.
//
// Path data is verbatim from `components/icon/material/data/mdi-7.4.47.js`
// in the WaterUI repository (Material Design Icons 7.4.47, 24×24 viewBox).

import SwiftUI

#if os(iOS)
  import UIKit
#else
  import AppKit
#endif

enum MaterialIcon {
  case inbox, imageAlbum, viewGallery, cog, pencil, calendarToday, calendarClock, flag, bellAlert, check, formatListBulleted, plus, circleOutline, album

  /// The `d` attribute of the icon's single `<path>`.
  var pathData: String {
    switch self {
    case .inbox:
      "M19,15H15A3,3 0 0,1 12,18A3,3 0 0,1 9,15H5V5H19M19,3H5C3.89,3 3,3.9 3,5V19A2,2 0 0,0 5,21H19A2,2 0 0,0 21,19V5A2,2 0 0,0 19,3Z"
    case .imageAlbum:
      "M6,19L9,15.14L11.14,17.72L14.14,13.86L18,19H6M6,4H11V12L8.5,10.5L6,12M18,2H6A2,2 0 0,0 4,4V20A2,2 0 0,0 6,22H18A2,2 0 0,0 20,20V4A2,2 0 0,0 18,2Z"
    case .viewGallery:
      "M21 3H2V16H21V3M2 17H6V21H2V17M7 17H11V21H7V17M12 17H16V21H12V17M17 17H21V21H17V17Z"
    case .cog:
      "M12,15.5A3.5,3.5 0 0,1 8.5,12A3.5,3.5 0 0,1 12,8.5A3.5,3.5 0 0,1 15.5,12A3.5,3.5 0 0,1 12,15.5M19.43,12.97C19.47,12.65 19.5,12.33 19.5,12C19.5,11.67 19.47,11.34 19.43,11L21.54,9.37C21.73,9.22 21.78,8.95 21.66,8.73L19.66,5.27C19.54,5.05 19.27,4.96 19.05,5.05L16.56,6.05C16.04,5.66 15.5,5.32 14.87,5.07L14.5,2.42C14.46,2.18 14.25,2 14,2H10C9.75,2 9.54,2.18 9.5,2.42L9.13,5.07C8.5,5.32 7.96,5.66 7.44,6.05L4.95,5.05C4.73,4.96 4.46,5.05 4.34,5.27L2.34,8.73C2.21,8.95 2.27,9.22 2.46,9.37L4.57,11C4.53,11.34 4.5,11.67 4.5,12C4.5,12.33 4.53,12.65 4.57,12.97L2.46,14.63C2.27,14.78 2.21,15.05 2.34,15.27L4.34,18.73C4.46,18.95 4.73,19.03 4.95,18.95L7.44,17.94C7.96,18.34 8.5,18.68 9.13,18.93L9.5,21.58C9.54,21.82 9.75,22 10,22H14C14.25,22 14.46,21.82 14.5,21.58L14.87,18.93C15.5,18.67 16.04,18.34 16.56,17.94L19.05,18.95C19.27,19.03 19.54,18.95 19.66,18.73L21.66,15.27C21.78,15.05 21.73,14.78 21.54,14.63L19.43,12.97Z"
    case .pencil:
      "M20.71,7.04C21.1,6.65 21.1,6 20.71,5.63L18.37,3.29C18,2.9 17.35,2.9 16.96,3.29L15.12,5.12L18.87,8.87M3,17.25V21H6.75L17.81,9.93L14.06,6.18L3,17.25Z"
    case .calendarToday:
      "M7,10H12V15H7M19,19H5V8H19M19,3H18V1H16V3H8V1H6V3H5C3.89,3 3,3.9 3,5V19A2,2 0 0,0 5,21H19A2,2 0 0,0 21,19V5A2,2 0 0,0 19,3Z"
    case .calendarClock:
      "M15,13H16.5V15.82L18.94,17.23L18.19,18.53L15,16.69V13M19,8H5V19H9.67C9.24,18.09 9,17.07 9,16A7,7 0 0,1 16,9C17.07,9 18.09,9.24 19,9.67V8M5,21C3.89,21 3,20.1 3,19V5C3,3.89 3.89,3 5,3H6V1H8V3H16V1H18V3H19A2,2 0 0,1 21,5V11.1C22.24,12.36 23,14.09 23,16A7,7 0 0,1 16,23C14.09,23 12.36,22.24 11.1,21H5M16,11.15A4.85,4.85 0 0,0 11.15,16C11.15,18.68 13.32,20.85 16,20.85A4.85,4.85 0 0,0 20.85,16C20.85,13.32 18.68,11.15 16,11.15Z"
    case .flag:
      "M14.4,6L14,4H5V21H7V14H12.6L13,16H20V6H14.4Z"
    case .bellAlert:
      "M23 7V13H21V7M21 15H23V17H21M12 2A2 2 0 0 0 10 4A2 2 0 0 0 10 4.29C7.12 5.14 5 7.82 5 11V17L3 19V20H21V19L19 17V11C19 7.82 16.88 5.14 14 4.29A2 2 0 0 0 14 4A2 2 0 0 0 12 2M10 21A2 2 0 0 0 12 23A2 2 0 0 0 14 21Z"
    case .check:
      "M21,7L9,19L3.5,13.5L4.91,12.09L9,16.17L19.59,5.59L21,7Z"
    case .formatListBulleted:
      "M7,5H21V7H7V5M7,13V11H21V13H7M4,4.5A1.5,1.5 0 0,1 5.5,6A1.5,1.5 0 0,1 4,7.5A1.5,1.5 0 0,1 2.5,6A1.5,1.5 0 0,1 4,4.5M4,10.5A1.5,1.5 0 0,1 5.5,12A1.5,1.5 0 0,1 4,13.5A1.5,1.5 0 0,1 2.5,12A1.5,1.5 0 0,1 4,10.5M7,19V17H21V19H7M4,16.5A1.5,1.5 0 0,1 5.5,18A1.5,1.5 0 0,1 4,19.5A1.5,1.5 0 0,1 2.5,18A1.5,1.5 0 0,1 4,16.5Z"
    case .plus:
      "M19,13H13V19H11V13H5V11H11V5H13V11H19V13Z"
    case .circleOutline:
      "M12,20A8,8 0 0,1 4,12A8,8 0 0,1 12,4A8,8 0 0,1 20,12A8,8 0 0,1 12,20M12,2A10,10 0 0,0 2,12A10,10 0 0,0 12,22A10,10 0 0,0 22,12A10,10 0 0,0 12,2Z"
    case .album:
      "M12,11A1,1 0 0,0 11,12A1,1 0 0,0 12,13A1,1 0 0,0 13,12A1,1 0 0,0 12,11M12,16.5C9.5,16.5 7.5,14.5 7.5,12C7.5,9.5 9.5,7.5 12,7.5C14.5,7.5 16.5,9.5 16.5,12C16.5,14.5 14.5,16.5 12,16.5M12,2A10,10 0 0,0 2,12A10,10 0 0,0 12,22A10,10 0 0,0 22,12A10,10 0 0,0 12,2Z"
    }
  }

  /// The icon as a filled shape in the current foreground style, framed to
  /// `size` points a side — `Svg` at its intrinsic 24pt, or `.size(w, h)`.
  func view(size: CGFloat = 24) -> some View {
    SvgPathShape(pathData: pathData, viewBox: 24)
      .fill(.foreground)
      .frame(width: size, height: size)
  }

  #if os(iOS)
    /// The icon rasterised as a template image `size` points a side, for the
    /// places SwiftUI accepts only an `Image` (tab items, bar buttons).
    func templateImage(size: CGFloat = 24) -> Image {
      let bounds = CGRect(x: 0, y: 0, width: size, height: size)
      let renderer = UIGraphicsImageRenderer(bounds: bounds)
      let image = renderer.image { _ in
        UIColor.black.setFill()
        UIBezierPath(cgPath: SvgPathShape(pathData: pathData, viewBox: 24).path(in: bounds).cgPath)
          .fill()
      }
      return Image(uiImage: image.withRenderingMode(.alwaysTemplate))
    }
  #else
    func templateImage(size: CGFloat = 24) -> Image {
      let bounds = CGRect(x: 0, y: 0, width: size, height: size)
      let image = NSImage(size: bounds.size, flipped: true) { _ in
        NSColor.black.setFill()
        NSBezierPath(cgPath: SvgPathShape(pathData: pathData, viewBox: 24).path(in: bounds).cgPath)
          .fill()
        return true
      }
      image.isTemplate = true
      return Image(nsImage: image)
    }
  #endif
}

/// An SVG path (`d` attribute) as a SwiftUI shape, scaled from its viewBox
/// into the rect it is drawn in. Implements the SVG 1.1 path grammar: every
/// command in absolute and relative form, implicit command repetition, and
/// elliptical arcs converted through the centre parameterisation.
struct SvgPathShape: Shape {
  let pathData: String
  let viewBox: CGFloat

  func path(in rect: CGRect) -> Path {
    var builder = SvgPathBuilder()
    builder.parse(pathData)
    let scale = min(rect.width, rect.height) / viewBox
    return builder.path.applying(
      CGAffineTransform(translationX: rect.minX, y: rect.minY).scaledBy(x: scale, y: scale))
  }
}

private struct SvgPathBuilder {
  var path = Path()
  private var current = CGPoint.zero
  private var subpathStart = CGPoint.zero
  private var lastControl: CGPoint?
  private var lastCommand: Character = "M"

  mutating func parse(_ data: String) {
    var scanner = SvgScanner(data)
    var command: Character = "M"
    while let token = scanner.nextCommandOrNumber() {
      switch token {
      case .command(let c):
        command = c
      case .number(let n):
        scanner.pushBack(n)
      }
      apply(command, &scanner)
      // A repeated implicit command after `M`/`m` is `L`/`l`.
      if command == "M" { command = "L" } else if command == "m" { command = "l" }
    }
  }

  private mutating func apply(_ command: Character, _ scanner: inout SvgScanner) {
    let relative = command.isLowercase
    func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
      relative ? CGPoint(x: current.x + x, y: current.y + y) : CGPoint(x: x, y: y)
    }
    switch command.uppercased() {
    case "M":
      let p = point(scanner.number(), scanner.number())
      path.move(to: p)
      current = p
      subpathStart = p
      lastControl = nil
    case "L":
      let p = point(scanner.number(), scanner.number())
      path.addLine(to: p)
      current = p
      lastControl = nil
    case "H":
      let x = scanner.number()
      let p = CGPoint(x: relative ? current.x + x : x, y: current.y)
      path.addLine(to: p)
      current = p
      lastControl = nil
    case "V":
      let y = scanner.number()
      let p = CGPoint(x: current.x, y: relative ? current.y + y : y)
      path.addLine(to: p)
      current = p
      lastControl = nil
    case "C":
      let c1 = point(scanner.number(), scanner.number())
      let c2 = point(scanner.number(), scanner.number())
      let p = point(scanner.number(), scanner.number())
      path.addCurve(to: p, control1: c1, control2: c2)
      current = p
      lastControl = c2
    case "S":
      let c1 = reflectedControl(for: "C")
      let c2 = point(scanner.number(), scanner.number())
      let p = point(scanner.number(), scanner.number())
      path.addCurve(to: p, control1: c1, control2: c2)
      current = p
      lastControl = c2
    case "Q":
      let c = point(scanner.number(), scanner.number())
      let p = point(scanner.number(), scanner.number())
      path.addQuadCurve(to: p, control: c)
      current = p
      lastControl = c
    case "T":
      let c = reflectedControl(for: "Q")
      let p = point(scanner.number(), scanner.number())
      path.addQuadCurve(to: p, control: c)
      current = p
      lastControl = c
    case "A":
      let rx = scanner.number()
      let ry = scanner.number()
      let rotation = scanner.number()
      let largeArc = scanner.number() != 0
      let sweep = scanner.number() != 0
      let p = point(scanner.number(), scanner.number())
      addArc(to: p, rx: rx, ry: ry, rotationDegrees: rotation, largeArc: largeArc, sweep: sweep)
      current = p
      lastControl = nil
    case "Z":
      path.closeSubpath()
      current = subpathStart
      lastControl = nil
    default:
      fatalError("SvgPathShape: unsupported path command '\(command)'")
    }
    lastCommand = Character(command.uppercased())
  }

  /// The reflection of the previous control point through the current
  /// point, or the current point when the previous command was not of the
  /// same family (SVG 1.1 §8.3.6 / §8.3.7).
  private func reflectedControl(for family: Character) -> CGPoint {
    guard let control = lastControl, lastCommand == family || lastCommand == (family == "C" ? "S" : "T")
    else { return current }
    return CGPoint(x: 2 * current.x - control.x, y: 2 * current.y - control.y)
  }

  /// SVG 1.1 implementation notes F.6.5: endpoint to centre parameterisation.
  private mutating func addArc(
    to end: CGPoint, rx: CGFloat, ry: CGFloat, rotationDegrees: CGFloat, largeArc: Bool, sweep: Bool
  ) {
    if current == end { return }
    if rx == 0 || ry == 0 {
      path.addLine(to: end)
      return
    }
    let phi = rotationDegrees * .pi / 180
    let cosPhi = cos(phi)
    let sinPhi = sin(phi)
    let dx = (current.x - end.x) / 2
    let dy = (current.y - end.y) / 2
    let x1 = cosPhi * dx + sinPhi * dy
    let y1 = -sinPhi * dx + cosPhi * dy
    var rx = abs(rx)
    var ry = abs(ry)
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry)
    if lambda > 1 {
      rx *= sqrt(lambda)
      ry *= sqrt(lambda)
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1
    var coefficient = sqrt(max(0, numerator / denominator))
    if largeArc == sweep { coefficient = -coefficient }
    let cx1 = coefficient * rx * y1 / ry
    let cy1 = -coefficient * ry * x1 / rx
    let cx = cosPhi * cx1 - sinPhi * cy1 + (current.x + end.x) / 2
    let cy = sinPhi * cx1 + cosPhi * cy1 + (current.y + end.y) / 2
    func angle(_ ux: CGFloat, _ uy: CGFloat, _ vx: CGFloat, _ vy: CGFloat) -> CGFloat {
      let dot = ux * vx + uy * vy
      let length = sqrt(ux * ux + uy * uy) * sqrt(vx * vx + vy * vy)
      var value = acos(max(-1, min(1, dot / length)))
      if ux * vy - uy * vx < 0 { value = -value }
      return value
    }
    let startAngle = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry)
    var delta = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry)
    if !sweep && delta > 0 { delta -= 2 * .pi }
    if sweep && delta < 0 { delta += 2 * .pi }
    let transform = CGAffineTransform(translationX: cx, y: cy).rotated(by: phi).scaledBy(x: rx, y: ry)
    path.addRelativeArc(
      center: .zero, radius: 1, startAngle: .radians(startAngle), delta: .radians(delta),
      transform: transform)
  }
}

/// Tokenises SVG path data: single-letter commands and numbers separated by
/// whitespace, commas, or nothing at all (`10-5`, `.5.5`, `1e-3`).
private struct SvgScanner {
  private let characters: [Character]
  private var index = 0
  private var pushedBack: CGFloat?

  enum Token {
    case command(Character)
    case number(CGFloat)
  }

  init(_ data: String) {
    characters = Array(data)
  }

  mutating func pushBack(_ value: CGFloat) {
    pushedBack = value
  }

  mutating func nextCommandOrNumber() -> Token? {
    if let value = pushedBack {
      pushedBack = nil
      return .number(value)
    }
    skipSeparators()
    guard index < characters.count else { return nil }
    let c = characters[index]
    if c.isLetter && c != "e" && c != "E" {
      index += 1
      return .command(c)
    }
    return .number(scanNumber())
  }

  mutating func number() -> CGFloat {
    if let value = pushedBack {
      pushedBack = nil
      return value
    }
    skipSeparators()
    precondition(index < characters.count, "SvgPathShape: path data ends inside a command")
    return scanNumber()
  }

  private mutating func skipSeparators() {
    while index < characters.count, characters[index] == "," || characters[index].isWhitespace {
      index += 1
    }
  }

  private mutating func scanNumber() -> CGFloat {
    var text = ""
    var seenDot = false
    var seenExponent = false
    if characters[index] == "-" || characters[index] == "+" {
      text.append(characters[index])
      index += 1
    }
    while index < characters.count {
      let c = characters[index]
      if c.isNumber {
        text.append(c)
      } else if c == "." && !seenDot && !seenExponent {
        seenDot = true
        text.append(c)
      } else if (c == "e" || c == "E") && !seenExponent {
        seenExponent = true
        text.append(c)
        if index + 1 < characters.count, characters[index + 1] == "-" || characters[index + 1] == "+" {
          index += 1
          text.append(characters[index])
        }
      } else {
        break
      }
      index += 1
    }
    guard let value = Double(text) else {
      fatalError("SvgPathShape: malformed number '\(text)' in path data")
    }
    return CGFloat(value)
  }
}
