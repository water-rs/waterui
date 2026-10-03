// Twin of examples/image: generated test pattern, remote photo with blur
// slider, and the custom URL loader section.
//
// The `Image::new` test pattern is deterministic RGBA data — the twin
// regenerates the identical pixels. The Photo sections load picsum.photos,
// which returns a different random image per request, so those regions can
// never pixel-match; the twin uses AsyncImage at the same geometry to keep
// layout parity.

import CoreGraphics
import SwiftUI

private func testPattern(_ width: Int, _ height: Int) -> CGImage? {
  var pixels = [UInt8](repeating: 0, count: width * height * 4)
  for y in 0 ..< height {
    for x in 0 ..< width {
      let i = (y * width + x) * 4
      pixels[i] = UInt8((Double(x) / Double(width)) * 255)
      pixels[i + 1] = UInt8((Double(y) / Double(height)) * 255)
      pixels[i + 2] = UInt8((Double(x + y) / Double(width + height)) * 255)
      pixels[i + 3] = 255
    }
  }
  return pixels.withUnsafeBytes { buf in
    guard let base = buf.baseAddress else { return nil }
    let data = Data(bytes: base, count: buf.count) as CFData
    guard let provider = CGDataProvider(data: data) else { return nil }
    return CGImage(
      width: width, height: height,
      bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
      provider: provider, decode: nil, shouldInterpolate: false,
      intent: .defaultIntent
    )
  }
}

struct ImageTwin: View {
  @State private var blur1 = 2.0
  @State private var blur2 = 0.0
  @State private var url = "https://picsum.photos/300/200"

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 10) {
        Text("GPU Image Processing").font(.title)
        Text("Demonstrating filtrate GPU filters on Image and Photo")
        wuiDivider()
        Text("Image Test").font(.headline)
        originalSection
        wuiDivider()
        Text("Photo Test").font(.headline)
        photoSection
        wuiDivider()
        customUrlSection
      }
      .padding(14)
    }
  }

  private var originalSection: some View {
    VStack(alignment: .leading, spacing: 10) {
      Text("Original Image").font(.headline)
      Text("No filters applied")
      if let img = testPattern(200, 150) {
        Image(decorative: img, scale: 1)
          .frame(width: 200, height: 150)
      }
    }
    .padding(14)
  }

  private var photoSection: some View {
    VStack(alignment: .leading, spacing: 10) {
      Text("Photo from URL").font(.headline)
      Text("Loading...").font(.subheadline)
      AsyncImage(url: URL(string: "https://picsum.photos/200/150")) { img in
        img.resizable().blur(radius: blur1)
      } placeholder: {
        Color.clear
      }
      .frame(width: 200, height: 150)
      HStack(spacing: 10) {
        Text("Blur:")
        Slider(value: $blur1, in: 0 ... 10).labelsHidden()
        Text(String(format: "%.1f", blur1))
      }
    }
    .padding(14)
  }

  private var customUrlSection: some View {
    VStack(alignment: .leading, spacing: 10) {
      Text("Custom URL Loader").font(.headline)
      Text("Load any image from the web (blur updates in real-time)")
      HStack(spacing: 10) {
        TextField("Image URL", text: $url)
          .labelsHidden()
          .textFieldStyle(.roundedBorder)
          .overlay(alignment: .leading) {
            if url.isEmpty { Text("Enter image URL").foregroundStyle(.secondary).allowsHitTesting(false) }
          }
        Button("Load") {}.buttonStyle(.borderedProminent)
      }
      Text("Enter a URL and click Load").font(.subheadline)
      Text("No image loaded yet").foregroundStyle(.gray)
      HStack(spacing: 10) {
        Text("Blur:")
        Slider(value: $blur2, in: 0 ... 20).labelsHidden()
        Text(String(format: "%.1f", blur2))
      }
    }
    .padding(14)
  }
}
