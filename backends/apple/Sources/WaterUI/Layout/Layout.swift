import CWaterUI
import SwiftUI

struct WuiProposalSize {
    var width: Float?
    var height: Float?

    init(width: Float? = nil, height: Float? = nil) {
        self.width = width
        self.height = height
    }

    init(_ raw: CWaterUI.WuiProposalSize) {
        self.width = raw.width.isNaN ? nil : raw.width
        self.height = raw.height.isNaN ? nil : raw.height
    }

    init(_ proposal: ProposedViewSize) {
        self.width = proposal.width.map { Float($0) }
        self.height = proposal.height.map { Float($0) }
    }

    init(size: CGSize) {
        self.width = size.width.isNaN ? nil : Float(size.width)
        self.height = size.height.isNaN ? nil : Float(size.height)
    }

    func toCStruct() -> CWaterUI.WuiProposalSize {
        CWaterUI.WuiProposalSize(
            width: width ?? .nan,
            height: height ?? .nan
        )
    }

    func toProposedSize() -> ProposedViewSize {
        ProposedViewSize(
            width: width.map { CGFloat($0) },
            height: height.map { CGFloat($0) }
        )
    }
}

struct WuiPoint {
    var x: Float
    var y: Float

    init(_ point: CGPoint) {
        self.x = Float(point.x)
        self.y = Float(point.y)
    }

    init(_ raw: CWaterUI.WuiPoint) {
        self.x = raw.x
        self.y = raw.y
    }

    func toCStruct() -> CWaterUI.WuiPoint {
        CWaterUI.WuiPoint(x: x, y: y)
    }

    var cgPoint: CGPoint {
        CGPoint(x: CGFloat(x), y: CGFloat(y))
    }
}

struct WuiSize {
    var width: Float
    var height: Float

    init(width: Float, height: Float) {
        self.width = width
        self.height = height
    }

    init(_ size: CGSize) {
        self.width = Float(size.width)
        self.height = Float(size.height)
    }

    init(_ raw: CWaterUI.WuiSize) {
        self.width = raw.width
        self.height = raw.height
    }

    func toCStruct() -> CWaterUI.WuiSize {
        CWaterUI.WuiSize(width: width, height: height)
    }

    var cgSize: CGSize {
        CGSize(width: CGFloat(width), height: CGFloat(height))
    }
}

struct WuiRect {
    var origin: WuiPoint
    var size: WuiSize

    init(_ rect: CGRect) {
        self.origin = WuiPoint(rect.origin)
        self.size = WuiSize(rect.size)
    }

    init(_ raw: CWaterUI.WuiRect) {
        self.origin = WuiPoint(raw.origin)
        self.size = WuiSize(raw.size)
    }

    func toCStruct() -> CWaterUI.WuiRect {
        CWaterUI.WuiRect(origin: origin.toCStruct(), size: size.toCStruct())
    }

    var cgRect: CGRect {
        CGRect(origin: origin.cgPoint, size: size.cgSize)
    }
}

struct WuiChildMetadata {
    var proposal: WuiProposalSize
    var priority: UInt8
    var stretch: Bool

    init(proposal: WuiProposalSize = .init(), priority: UInt8 = 0, stretch: Bool = false) {
        self.proposal = proposal
        self.priority = priority
        self.stretch = stretch
    }

    init(_ raw: CWaterUI.WuiChildMetadata) {
        self.proposal = WuiProposalSize(raw.proposal)
        self.priority = raw.priority
        self.stretch = raw.stretch
    }

    func toCStruct() -> CWaterUI.WuiChildMetadata {
        CWaterUI.WuiChildMetadata(
            proposal: proposal.toCStruct(),
            priority: priority,
            stretch: stretch
        )
    }
}

@MainActor
final class WuiLayout {
    private var inner: OpaquePointer

    init(inner: OpaquePointer) {
        self.inner = inner
    }

    @MainActor deinit {
        waterui_drop_layout(inner)
    }

    func propose(parent: WuiProposalSize, children: [WuiChildMetadata]) -> [WuiProposalSize] {
            let childArray = WuiArray(array: children.map { $0.toCStruct() })
            let parentRaw = parent.toCStruct()

        let typedChildren = unsafeBitCast(
            childArray.inner.intoInner(),
            to: CWaterUI.WuiArray_WuiChildMetadata.self
        )
        let proposals = waterui_layout_propose(inner, parentRaw, typedChildren)
        let rawArray = unsafeBitCast(proposals, to: CWaterUI.WuiArray.self)
        let bridged = WuiArray<CWaterUI.WuiProposalSize>(c: rawArray)
        return bridged.toArray().map { WuiProposalSize($0) }
        
    }

    func size(parent: WuiProposalSize, children: [WuiChildMetadata]) -> CGSize {
        let childArray = WuiArray(array: children.map { $0.toCStruct() })
        let parentRaw = parent.toCStruct()

        let typedChildren = unsafeBitCast(
            childArray.inner.intoInner(),
            to: CWaterUI.WuiArray_WuiChildMetadata.self
        )
        let size = waterui_layout_size(inner, parentRaw, typedChildren)
        return WuiSize(size).cgSize
    }

    func place(bound: CGRect, proposal: WuiProposalSize, children: [WuiChildMetadata]) -> [CGRect] {
        let childArray = WuiArray(array: children.map { $0.toCStruct() })
        let boundRaw = WuiRect(bound).toCStruct()
        let proposalRaw = proposal.toCStruct()

            let typedChildren = unsafeBitCast(
                childArray.inner.intoInner(),
                to: CWaterUI.WuiArray_WuiChildMetadata.self
            )
            let rects = waterui_layout_place(inner, boundRaw, proposalRaw, typedChildren)
            let rawArray = unsafeBitCast(rects, to: CWaterUI.WuiArray.self)
            let bridged = WuiArray<CWaterUI.WuiRect>(c: rawArray)
            return bridged.toArray().map { WuiRect($0).cgRect }
        }
    
}

@MainActor
struct WuiLayoutContainer: WuiComponent, View {
    static let id: String = Self.decodeId(waterui_layout_container_id())

    private var layout: WuiLayout
    private var children: [WuiAnyView]
    private var descriptors: [ChildDescriptor]

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        let container = waterui_force_as_layout_container(anyview)
        self.layout = WuiLayout(inner: container.layout!)

        let anyViews = WuiAnyViews(container.contents)
        var children: [WuiAnyView] = []
        children.reserveCapacity(anyViews.count)
        for i in 0..<anyViews.count {
            children.append(anyViews.getView(at: i, env: env))
        }
        self.children = children

        self.descriptors = children.map { view in
            let id = view.typeId
            return ChildDescriptor(typeId: id, isSpacer: id == WuiSpacer.id)
        }
    }

    var body: some View {
        RustLayout(layout: layout, descriptors: descriptors) {
            ForEach(children) { child in
                child
            }
        }
    }
}

@MainActor
struct WuiFixedContainer: WuiComponent, View {
    static let id: String = Self.decodeId(waterui_fixed_container_id())

    private var layout: WuiLayout
    private var children: [WuiAnyView]
    private var descriptors: [ChildDescriptor]

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        let container = waterui_force_as_fixed_container(anyview)
        self.layout = WuiLayout(inner: container.layout!)

        let pointerArray = WuiArray<OpaquePointer>(container.contents)
        let resolvedChildren = pointerArray
            .toArray()
            .map { WuiAnyView(anyview: $0, env: env) }

        self.children = resolvedChildren
        self.descriptors = resolvedChildren.map { view in
            let id = view.typeId
            return ChildDescriptor(typeId: id, isSpacer: id == WuiSpacer.id)
        }
    }

    var body: some View {
        RustLayout(layout: layout, descriptors: descriptors) {
            ForEach(children) { child in
                child
            }
        }
    }
}

private struct ChildDescriptor {
    let typeId: String
    let isSpacer: Bool
}

private struct RustLayout: @preconcurrency Layout {
    private var layout: WuiLayout
    private var descriptors: [ChildDescriptor]

    init(layout: WuiLayout, descriptors: [ChildDescriptor]) {
        self.layout = layout
        self.descriptors = descriptors
    }

    // --- Cache remains the same ---
    struct Cache {
        var metadata: [WuiChildMetadata] = []
    }

    func makeCache(subviews: Subviews) -> Cache {
        Cache()
    }
    
    func updateCache(_ cache: inout Cache, subviews: Subviews) {
        // Cache is generated on-demand in sizeThatFits
    }

    // --- MAJOR REFACTOR HERE ---
    @MainActor func sizeThatFits(
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout Cache
    ) -> CGSize {
        let parentProposal = WuiProposalSize(proposal)

        var metadata: [WuiChildMetadata] = []
        metadata.reserveCapacity(subviews.count)

        // First, create metadata for an initial proposal pass in Rust
        // This pass determines how much space to offer each child.
        for index in subviews.indices {
            let isStretchy = descriptors[safe: index]?.isSpacer ?? false
            metadata.append(
                WuiChildMetadata(
                    // We don't know the proposal size yet, so we send an empty one.
                    // The `stretch` flag is the most important info for this pass.
                    proposal: WuiProposalSize(),
                    priority: priorityValue(from: subviews[index]),
                    stretch: isStretchy
                )
            )
        }
        
        let childProposals = layout.propose(parent: parentProposal, children: metadata)

        // Now, measure children with the proposals from Rust and create the final metadata.
        metadata.removeAll(keepingCapacity: true)
        for index in subviews.indices {
            let subview = subviews[index]
            let isStretchy = descriptors[safe: index]?.isSpacer ?? false
            
            let childProposal = childProposals[safe: index] ?? WuiProposalSize()
            let swiftUIProposal = childProposal.toProposedSize()
            
            let measuredSize = subview.sizeThatFits(swiftUIProposal)
            

            // --- THIS IS THE KEY COMMUNICATION ---
            // If the child is a flexible spacer, tell Rust it has zero intrinsic size.
            // Otherwise, tell Rust the actual measured size.
            let metadataProposal = isStretchy ? WuiProposalSize() : WuiProposalSize(size: measuredSize)
            
            metadata.append(
                WuiChildMetadata(
                    proposal: metadataProposal,
                    priority: priorityValue(from: subview),
                    stretch: isStretchy
                )
            )
        }

        // Store the final metadata in the cache for the `placeSubviews` pass.
        cache.metadata = metadata

        // Ask Rust for the final container size based on the definitive child metadata.
        let finalSize = layout.size(parent: parentProposal, children: metadata)
        
  

        return finalSize
    }

    // --- Minor change to use the cache correctly ---
    @MainActor func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout Cache
    ) {
        guard bounds.isValidForLayout else {
            print("Warning: RustLayout.placeSubviews received invalid bounds: \(bounds)")
            return
        }

        let parentProposal = WuiProposalSize(proposal)
        
        // Use the metadata we already computed and cached in `sizeThatFits`.
        let rects = layout.place(
            bound: bounds,
            proposal: parentProposal,
            children: cache.metadata
        )

        for index in subviews.indices {
            guard let rect = rects[safe: index], rect.isValidForLayout else {
                print("Warning: RustLayout received an invalid rect for subview at index \(index): \(rects[safe: index] ?? .zero). Skipping placement.")
                continue
            }

            let sizeProposal = ProposedViewSize(width: rect.width, height: rect.height)
            subviews[index].place(
                at: CGPoint(x: rect.minX, y: rect.minY),
                proposal: sizeProposal
            )
        }
    }

    private func priorityValue(from subview: LayoutSubviews.Element) -> UInt8 {
        let raw = subview.priority
        guard raw.isFinite, raw > 0 else { return 0 }
        let clamped = min(max(raw, 0.0), 255.0)
        return UInt8(clamped.rounded())
    }
}

private extension Array {
    subscript(safe index: Index) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}


extension CGFloat {
    /// Checks if the value is a valid, finite number suitable for layout calculations.
    var isValidForLayout: Bool {
        return !self.isNaN && !self.isInfinite
    }
}

extension CGRect {
    /// Checks if the rect's origin and size are composed of valid, finite numbers.
    var isValidForLayout: Bool {
        return origin.x.isValidForLayout &&
               origin.y.isValidForLayout &&
               size.width.isValidForLayout &&
               size.height.isValidForLayout
    }
}
