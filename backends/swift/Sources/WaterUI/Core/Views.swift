import CWaterUI
import SwiftUI
import Synchronization

// since swift require an isloated collection to do random access, but waterui run on main thread
// we put a buffer to store the loaded ids
final class WuiAnyViewCollection: RandomAccessCollection, Sendable {
    nonisolated let buffer: Mutex<[WuiId]> = Mutex([])

    nonisolated let views: WuiAnyViews

    init(_ views: WuiAnyViews) {
        self.views = views
    }

    var startIndex: Int {
        return 0
    }

    var endIndex: Int {
        return buffer.withLock { $0.count }
    }

    subscript(position: Int) -> WuiId {
        // if almost at the end, try load more
        if position >= buffer.withLock({ $0.count }) - 10 {
            Task {
                await loadMore()
            }
        }

        return buffer.withLock { $0[position] }
    }

    private func loadMore() async {
        let currentCount = buffer.withLock { $0.count }
        let views = views
        let totalCount = await Task { @MainActor in views.count }.value
        guard currentCount < totalCount else {
            return
        }
        let toLoad = Swift.min(20, totalCount - currentCount)
        var newIds: [WuiId] = []
        for i in 0..<toLoad {
            let id = await Task { @MainActor in views.getId(at: currentCount + i) }.value
            newIds.append(id)
        }
        buffer.withLock { $0.append(contentsOf: newIds) }

    }

    @MainActor
    func intoForEach(env: WuiEnvironment)
        -> SwiftUI.ForEach<EnumeratedSequence<WuiAnyViewCollection>, WuiId, some View>
    {
        return SwiftUI.ForEach(WuiAnyViewCollection(views).enumerated(), id: \.element) {
            index, id in
            self.views.getView(at: index, env: env).id(id)
        }
    }

}

@MainActor
final class WuiAnyViews {
    let id = UUID()
    private let inner: OpaquePointer

    init(_ inner: OpaquePointer) {
        self.inner = inner
    }

    @MainActor deinit {
        waterui_drop_anyviews(inner)
    }

    var count: Int {
        Int(waterui_anyviews_len(inner))
    }

    func getId(at index: Int) -> WuiId {
        let id = waterui_anyviews_get_id(inner, UInt(index))
        return id
    }

    func getView(at index: Int, env: WuiEnvironment) -> WuiAnyView {
        let ptr = waterui_anyviews_get_view(inner, UInt(index))
        return WuiAnyView(anyview: ptr!, env: env)
    }

    func intoForEach(env: WuiEnvironment)
        -> SwiftUI.ForEach<EnumeratedSequence<WuiAnyViewCollection>, WuiId, some View>
    {
        WuiAnyViewCollection(self).intoForEach(env: env)
    }

}

extension WuiWatcher_____WuiAnyViews: Watcher {
    typealias Output = WuiAnyViews

    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)

        let call: @convention(c) (UnsafeRawPointer?, OpaquePointer?, OpaquePointer?) -> Void =
            {
                data, value, metadata in
                callWrapper(data, WuiAnyViews(value!), metadata)
            }

        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }

        self.init(data: data, call: call, drop: drop)
    }
}
