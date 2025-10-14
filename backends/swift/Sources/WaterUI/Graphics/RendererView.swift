import CWaterUI
import Foundation
import SwiftUI

@MainActor
private final class RendererHandle: ObservableObject {
    let pointer: UnsafeMutablePointer<WuiRendererView>

    init(pointer: UnsafeMutablePointer<WuiRendererView>) {
        self.pointer = pointer
    }

    @MainActor deinit {
        waterui_drop_renderer_view(OpaquePointer(pointer))
    }
}

@MainActor
struct WuiRendererView: View, WuiComponent {
    static var id: WuiTypeId { waterui_renderer_view_id() }

    @StateObject private var handle: RendererHandle
    private let width: Int
    private let height: Int

    init(anyview: OpaquePointer, env _: WuiEnvironment) {
        let rawHandle = waterui_force_as_renderer_view(anyview)
        precondition(rawHandle != nil, "waterui_force_as_renderer_view returned null")
        let width = Int(waterui_renderer_view_width(rawHandle))
        let height = Int(waterui_renderer_view_height(rawHandle))
        self._handle = StateObject(
            wrappedValue: RendererHandle(pointer: UnsafeMutablePointer<WuiRendererView>(rawHandle!))
        )
        self.width = max(width, 0)
        self.height = max(height, 0)
    }

    var body: some View {
        RendererSurfaceView(handle: handle, width: width, height: height)
            .frame(width: CGFloat(width), height: CGFloat(height))
    }
}

@MainActor
private struct RendererSurfaceView: View {
    @ObservedObject var handle: RendererHandle
    let width: Int
    let height: Int
    @State private var image: CGImage?

    var body: some View {
        ZStack {
            if let image {
                Image(decorative: image, scale: 1, orientation: .up)
                    .resizable()
                    .interpolation(.none)
                    .aspectRatio(contentMode: .fill)
            } else {
                Color.clear
            }
        }
        .task(id: handle.pointer) {
            image = renderSnapshot()
        }
    }

    private func renderSnapshot() -> CGImage? {
        guard width > 0, height > 0 else { return nil }
        let stride = width * 4
        var buffer = Data(count: stride * height)
        guard
            waterui_renderer_view_preferred_format(OpaquePointer(handle.pointer))
                == WuiRendererBufferFormat_Rgba8888
        else {
            return nil
        }
        let rendered = buffer.withUnsafeMutableBytes { rawBuffer -> Bool in
            guard let baseAddress = rawBuffer.baseAddress else { return false }
            return waterui_renderer_view_render_cpu(
                OpaquePointer(handle.pointer),
                baseAddress.assumingMemoryBound(to: UInt8.self),
                UInt32(width),
                UInt32(height),
                UInt(stride),
                WuiRendererBufferFormat_Rgba8888
            )
        }
        guard rendered else { return nil }

        guard let provider = CGDataProvider(data: buffer as CFData) else {
            return nil
        }

        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
        return CGImage(
            width: width,
            height: height,
            bitsPerComponent: 8,
            bitsPerPixel: 32,
            bytesPerRow: stride,
            space: colorSpace,
            bitmapInfo: bitmapInfo,
            provider: provider,
            decode: nil,
            shouldInterpolate: true,
            intent: .defaultIntent
        )
    }
}
