import CWaterUI
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

}

@MainActor
final class WuiAnyViews {
    private let inner: OpaquePointer
    private let environment: WuiEnvironment

    init(_ inner: OpaquePointer, env: WuiEnvironment) {
        self.inner = inner
        self.environment = env
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

    func getView(at index: Int) -> WuiAnyView {
        let ptr = waterui_anyviews_get_view(inner, UInt(index))
        return WuiAnyView(anyview: ptr!, env: environment)
    }

}
