import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// Screenshot comparison for the e2e suite.
//
//   compare-screenshots.swift compare <expected.png> <actual.png> <diff.png>
//     Exits 0 when the fraction of pixels differing by more than `tolerance`
//     per channel stays under `budget`, else 1; always writes an amplified
//     diff to <diff.png> for the artifact bundle.
//   compare-screenshots.swift content <image.png>
//     Exits 0 when the image carries real content — i.e. it is not a single
//     near-uniform fill, which is what a crashed or never-presented render
//     produces.
//
// Env knobs: DIFF_TOLERANCE (per-channel, default 16/255), DIFF_BUDGET
// (fraction of pixels allowed to differ, default 0.02), CONTENT_BUDGET
// (fraction that must differ from the dominant fill, default 0.001).

enum CompareError: Error {
  case unreadable(String)
  case noContext
  case sizeMismatch(Int, Int, Int, Int)
}

func loadPixels(_ path: String) throws -> (Int, Int, [UInt8]) {
  let url = URL(fileURLWithPath: path) as CFURL
  guard
    let source = CGImageSourceCreateWithURL(url, nil),
    let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
  else {
    throw CompareError.unreadable(path)
  }
  let width = image.width
  let height = image.height
  var bytes = [UInt8](repeating: 0, count: width * height * 4)
  guard
    let context = CGContext(
      data: &bytes,
      width: width,
      height: height,
      bitsPerComponent: 8,
      bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )
  else {
    throw CompareError.noContext
  }
  context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
  return (width, height, bytes)
}

func writePNG(_ bytes: [UInt8], width: Int, height: Int, to path: String) throws {
  var bytes = bytes
  guard
    let context = CGContext(
      data: &bytes,
      width: width,
      height: height,
      bitsPerComponent: 8,
      bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ),
    let image = context.makeImage(),
    let destination = CGImageDestinationCreateWithURL(
      URL(fileURLWithPath: path) as CFURL, UTType.png.identifier as CFString, 1, nil)
  else {
    throw CompareError.noContext
  }
  CGImageDestinationAddImage(destination, image, nil)
  CGImageDestinationFinalize(destination)
}

func envDouble(_ name: String, _ fallback: Double) -> Double {
  ProcessInfo.processInfo.environment[name].flatMap(Double.init) ?? fallback
}

func differs(_ a: UInt8, _ b: UInt8, tolerance: Int) -> Bool {
  abs(Int(a) - Int(b)) > tolerance
}

do {
  let args = CommandLine.arguments
  guard args.count >= 3 else {
    FileHandle.standardError.write(
      Data(
        "usage: compare-screenshots.swift compare <expected> <actual> <diff> | content <image>\n"
          .utf8))
    exit(2)
  }

  switch args[1] {
  case "content":
    let (_, _, bytes) = try loadPixels(args[2])
    let tolerance = Int(envDouble("DIFF_TOLERANCE", 16))
    let budget = envDouble("CONTENT_BUDGET", 0.001)
    // Compare every pixel against the first one: a uniform fill is what a
    // blank or never-presented window looks like.
    let anchor = (bytes[0], bytes[1], bytes[2])
    var differing = 0
    for offset in stride(from: 4, to: bytes.count, by: 4) {
      if differs(bytes[offset], anchor.0, tolerance: tolerance)
        || differs(bytes[offset + 1], anchor.1, tolerance: tolerance)
        || differs(bytes[offset + 2], anchor.2, tolerance: tolerance)
      {
        differing += 1
      }
    }
    let ratio = Double(differing) / Double(bytes.count / 4)
    print(String(format: "content: %.4f of pixels differ from the dominant fill", ratio))
    exit(ratio > budget ? 0 : 1)

  case "compare":
    guard args.count == 5 else {
      FileHandle.standardError.write(
        Data("usage: compare-screenshots.swift compare <expected> <actual> <diff>\n".utf8))
      exit(2)
    }
    let (width, height, expected) = try loadPixels(args[2])
    let (actualWidth, actualHeight, actual) = try loadPixels(args[3])
    guard width == actualWidth, height == actualHeight else {
      throw CompareError.sizeMismatch(width, height, actualWidth, actualHeight)
    }
    let tolerance = Int(envDouble("DIFF_TOLERANCE", 16))
    let budget = envDouble("DIFF_BUDGET", 0.02)
    var differing = 0
    var diff = [UInt8](repeating: 255, count: expected.count)
    for offset in stride(from: 0, to: expected.count, by: 4) {
      let delta = (
        abs(Int(expected[offset]) - Int(actual[offset])),
        abs(Int(expected[offset + 1]) - Int(actual[offset + 1])),
        abs(Int(expected[offset + 2]) - Int(actual[offset + 2]))
      )
      if delta.0 > tolerance || delta.1 > tolerance || delta.2 > tolerance {
        differing += 1
        diff[offset] = UInt8(min(255, delta.0 * 4))
        diff[offset + 1] = UInt8(min(255, delta.1 * 4))
        diff[offset + 2] = UInt8(min(255, delta.2 * 4))
      } else {
        diff[offset] = 0
        diff[offset + 1] = 0
        diff[offset + 2] = 0
      }
    }
    try writePNG(diff, width: width, height: height, to: args[4])
    let ratio = Double(differing) / Double(width * height)
    print(
      String(
        format: "compare: %.4f of pixels differ (tolerance %d, budget %.4f)",
        ratio, tolerance, budget))
    exit(ratio <= budget ? 0 : 1)

  default:
    FileHandle.standardError.write(Data("unknown mode: \(args[1])\n".utf8))
    exit(2)
  }
} catch CompareError.unreadable(let path) {
  FileHandle.standardError.write(Data("cannot read image: \(path)\n".utf8))
  exit(1)
} catch CompareError.sizeMismatch(let w, let h, let aw, let ah) {
  FileHandle.standardError.write(
    Data("size mismatch: baseline \(w)x\(h) vs capture \(aw)x\(ah)\n".utf8))
  exit(1)
} catch {
  FileHandle.standardError.write(Data("\(error)\n".utf8))
  exit(1)
}
