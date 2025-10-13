//
//  Gesture.swift
//  WaterUI
//
//  Created by OpenAI Codex on 11/23/25.
//

import CWaterUI
import Foundation
import SwiftUI

@MainActor
struct WuiGestureView: WuiComponent, View {
    static var id: WuiTypeId {
        waterui_gesture_id()
    }

    private var content: WuiAnyView
    private var sequence: [BaseGestureDescriptor]
    private var action: GestureAction

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        let metadata = waterui_force_as_gesture(anyview)
        self.content = WuiAnyView(anyview: metadata.view, env: env)
        self.action = GestureAction(inner: metadata.action, env: env)

        if let gesturePtr = metadata.gesture {
            let descriptor = GestureDescriptor(pointer: gesturePtr)
            self.sequence = descriptor.flattened()
            waterui_drop_gesture(gesturePtr)
        } else {
            self.sequence = []
        }
    }

    var body: some View {
        if sequence.isEmpty {
            content
        } else {
            GestureSequenceContainer(content: content, sequence: sequence, action: action)
        }
    }
}

// MARK: - Gesture Sequence Container

@MainActor
private struct GestureSequenceContainer<Content: View>: View {
    let content: Content
    let sequence: [BaseGestureDescriptor]
    let action: GestureAction

    @State private var stage: Int = 0
    @State private var viewSize: CGSize = .zero
    @State private var dragState = DragTrackingState()
    @State private var magnificationState = MagnificationTrackingState()
    @State private var longPressState = LongPressTrackingState()

    var body: some View {
        let sizedContent = content.background(SizeReader(size: $viewSize))

        return Group {
            if let descriptor = sequence[safe: stage] {
                apply(descriptor: descriptor, to: sizedContent, stageIndex: stage)
            } else {
                sizedContent
            }
        }
    }

    @ViewBuilder
    private func apply<V: View>(descriptor: BaseGestureDescriptor, to view: V, stageIndex: Int) -> some View {
        let lastStageIndex = sequence.count - 1

        switch descriptor {
        case .tap(let tap):
            view.simultaneousGesture(
                SpatialTapGesture(count: Int(tap.count), coordinateSpace: .local)
                    .onEnded { value in
                        guard stage == stageIndex else { return }
                        if stageIndex == lastStageIndex {
                            action.call(
                                kind: WuiGestureEventKind_Tap,
                                phase: WuiGesturePhase_Ended,
                                location: value.location,
                                count: tap.count
                            )
                        }
                        advanceStage(from: stageIndex)
                    }
            )

        case .longPress(let press):
            let tracked = view.simultaneousGesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .local)
                    .onChanged { value in
                        guard stage == stageIndex else { return }
                        longPressState.currentLocation = value.location
                    }
            )

            tracked.simultaneousGesture(
                LongPressGesture(minimumDuration: press.minimumDuration, maximumDistance: 10)
                    .onChanged { isPressing in
                        guard stage == stageIndex, isPressing else { return }
                        if !longPressState.isPressing {
                            longPressState.begin(at: longPressState.currentLocation)
                            if stageIndex == lastStageIndex {
                                action.call(
                                    kind: WuiGestureEventKind_LongPress,
                                    phase: WuiGesturePhase_Started,
                                    location: longPressState.currentLocation
                                )
                            }
                        }
                    }
                    .onEnded { success in
                        guard stage == stageIndex else { return }
                        defer { longPressState.reset() }

                        guard success else {
                            resetSequence()
                            return
                        }

                        let duration = Float(longPressState.duration)
                        let location = longPressState.location

                        if stageIndex == lastStageIndex {
                            action.call(
                                kind: WuiGestureEventKind_LongPress,
                                phase: WuiGesturePhase_Ended,
                                location: location,
                                duration: duration
                            )
                        }

                        advanceStage(from: stageIndex)
                    }
            )

        case .drag(let drag):
            view.simultaneousGesture(
                DragGesture(minimumDistance: CGFloat(drag.minDistance), coordinateSpace: .local)
                    .onChanged { value in
                        guard stage == stageIndex else { return }
                        let location = value.location
                        let translation = value.translation
                        let timestamp = value.time

                        if !dragState.isActive {
                            dragState.start(location: location, time: timestamp)
                            if stageIndex == lastStageIndex {
                                action.call(
                                    kind: WuiGestureEventKind_Drag,
                                    phase: WuiGesturePhase_Started,
                                    location: location,
                                    translation: translation
                                )
                            }
                            return
                        }

                        let velocity = dragState.velocity(to: location, at: timestamp)
                        dragState.update(location: location, time: timestamp)

                        if stageIndex == lastStageIndex {
                            action.call(
                                kind: WuiGestureEventKind_Drag,
                                phase: WuiGesturePhase_Updated,
                                location: location,
                                translation: translation,
                                velocity: velocity
                            )
                        }
                    }
                    .onEnded { value in
                        guard stage == stageIndex else { return }
                        let location = value.location
                        let translation = value.translation
                        let timestamp = value.time
                        let velocity = dragState.velocity(to: location, at: timestamp)

                        if stageIndex == lastStageIndex {
                            action.call(
                                kind: WuiGestureEventKind_Drag,
                                phase: WuiGesturePhase_Ended,
                                location: location,
                                translation: translation,
                                velocity: velocity
                            )
                        }

                        dragState.reset()
                        advanceStage(from: stageIndex)
                    }
            )

        case .magnification:
            view.simultaneousGesture(
                MagnificationGesture()
                    .onChanged { value in
                        guard stage == stageIndex else { return }
                        let now = Date()
                        if !magnificationState.isActive {
                            magnificationState.start(scale: value, time: now)
                            if stageIndex == lastStageIndex {
                                action.call(
                                    kind: WuiGestureEventKind_Magnification,
                                    phase: WuiGesturePhase_Started,
                                    location: centerPoint,
                                    scale: Float(value)
                                )
                            }
                            return
                        }

                        let velocity = magnificationState.velocity(to: value, at: now)
                        magnificationState.update(scale: value, time: now)

                        if stageIndex == lastStageIndex {
                            action.call(
                                kind: WuiGestureEventKind_Magnification,
                                phase: WuiGesturePhase_Updated,
                                location: centerPoint,
                                scale: Float(value),
                                velocityScalar: Float(velocity)
                            )
                        }
                    }
                    .onEnded { value in
                        guard stage == stageIndex else { return }
                        let now = Date()
                        let velocity = magnificationState.velocity(to: value, at: now)

                        if stageIndex == lastStageIndex {
                            action.call(
                                kind: WuiGestureEventKind_Magnification,
                                phase: WuiGesturePhase_Ended,
                                location: centerPoint,
                                scale: Float(value),
                                velocityScalar: Float(velocity)
                            )
                        }

                        magnificationState.reset()
                        advanceStage(from: stageIndex)
                    }
            )

        case .rotation:
            view
        }
    }

    private var centerPoint: CGPoint {
        CGPoint(x: viewSize.width / 2, y: viewSize.height / 2)
    }

    private func advanceStage(from index: Int) {
        if index >= sequence.count - 1 {
            stage = 0
        } else {
            stage = index + 1
        }
        dragState.reset()
        magnificationState.reset()
        longPressState.reset()
    }

    private func resetSequence() {
        stage = 0
        dragState.reset()
        magnificationState.reset()
        longPressState.reset()
    }
}

// MARK: - Size Reader
private struct SizePreferenceKey: PreferenceKey {
    static let defaultValue: CGSize = .zero
    static func reduce(value: inout CGSize, nextValue: () -> CGSize) {
        let next = nextValue()
        if next != .zero {
            value = next
        }
    }
}

private struct SizeReader: View {
    @Binding var size: CGSize

    var body: some View {
        GeometryReader { proxy in
            Color.clear
                .preference(key: SizePreferenceKey.self, value: proxy.size)
        }
        .onPreferenceChange(SizePreferenceKey.self) { value in
            if value != .zero {
                size = value
            }
        }
    }
}

// MARK: - Tracking State

private struct DragTrackingState {
    var isActive: Bool = false
    private var lastLocation: CGPoint = .zero
    private var lastTimestamp: Date = .distantPast

    mutating func start(location: CGPoint, time: Date) {
        isActive = true
        lastLocation = location
        lastTimestamp = time
    }

    mutating func update(location: CGPoint, time: Date) {
        lastLocation = location
        lastTimestamp = time
    }

    mutating func reset() {
        isActive = false
        lastLocation = .zero
        lastTimestamp = .distantPast
    }

    func velocity(to location: CGPoint, at time: Date) -> CGSize {
        guard isActive else { return .zero }
        let dt = time.timeIntervalSince(lastTimestamp)
        guard dt > 0 else { return .zero }
        let dx = location.x - lastLocation.x
        let dy = location.y - lastLocation.y
        return CGSize(width: dx / dt, height: dy / dt)
    }
}

private struct MagnificationTrackingState {
    var isActive: Bool = false
    private var lastScale: CGFloat = 1
    private var lastTimestamp: Date = .distantPast

    mutating func start(scale: CGFloat, time: Date) {
        isActive = true
        lastScale = scale
        lastTimestamp = time
    }

    mutating func update(scale: CGFloat, time: Date) {
        lastScale = scale
        lastTimestamp = time
    }

    mutating func reset() {
        isActive = false
        lastScale = 1
        lastTimestamp = .distantPast
    }

    func velocity(to scale: CGFloat, at time: Date) -> CGFloat {
        guard isActive else { return 0 }
        let dt = time.timeIntervalSince(lastTimestamp)
        guard dt > 0 else { return 0 }
        return (scale - lastScale) / CGFloat(dt)
    }
}

private struct LongPressTrackingState {
    var isPressing: Bool = false
    private(set) var startTime: Date?
    private(set) var location: CGPoint = .zero
    var currentLocation: CGPoint = .zero

    mutating func begin(at location: CGPoint) {
        isPressing = true
        startTime = Date()
        self.location = location
    }

    mutating func reset() {
        isPressing = false
        startTime = nil
        location = .zero
        currentLocation = .zero
    }

    var duration: TimeInterval {
        guard let startTime else { return 0 }
        return Date().timeIntervalSince(startTime)
    }
}

// MARK: - Gesture Action Wrapper

@MainActor
final class GestureAction {
    private var inner: OpaquePointer?
    private unowned let env: WuiEnvironment

    init(inner: OpaquePointer?, env: WuiEnvironment) {
        self.inner = inner
        self.env = env
    }

    func call(
        kind: WuiGestureEventKind,
        phase: WuiGesturePhase,
        location: CGPoint,
        translation: CGSize = .zero,
        velocity: CGSize = .zero,
        scale: Float = 1,
        velocityScalar: Float = 0,
        count: UInt32 = 0,
        duration: Float = 0
    ) {
        guard let inner else { return }

        let event = WuiGestureEvent(
            kind: kind,
            phase: phase,
            location: WuiGesturePoint(x: Float(location.x), y: Float(location.y)),
            translation: WuiGesturePoint(
                x: Float(translation.width),
                y: Float(translation.height)
            ),
            velocity: WuiGesturePoint(
                x: Float(velocity.width),
                y: Float(velocity.height)
            ),
            scale: scale,
            velocity_scalar: velocityScalar,
            count: count,
            duration: duration
        )

        waterui_call_gesture_action(inner, env.inner, event)
    }

    @MainActor deinit {
        if let inner {
            waterui_drop_action(inner)
        }
    }
}

// MARK: - Gesture Descriptor Bridging

private struct TapGestureDescriptor {
    let count: UInt32
}

private struct LongPressGestureDescriptor {
    let duration: UInt32

    var minimumDuration: Double {
        Double(duration) / 1000.0
    }
}

private struct DragGestureDescriptor {
    let minDistance: Float
}

private struct MagnificationGestureDescriptor {
    let initialScale: Float
}

private struct RotationGestureDescriptor {
    let initialAngle: Float
}

private enum BaseGestureDescriptor {
    case tap(TapGestureDescriptor)
    case longPress(LongPressGestureDescriptor)
    case drag(DragGestureDescriptor)
    case magnification(MagnificationGestureDescriptor)
    case rotation(RotationGestureDescriptor)
}

private enum GestureDescriptor {
    case tap(TapGestureDescriptor)
    case longPress(LongPressGestureDescriptor)
    case drag(DragGestureDescriptor)
    case magnification(MagnificationGestureDescriptor)
    case rotation(RotationGestureDescriptor)
    indirect case sequence(first: GestureDescriptor, then: GestureDescriptor)

    init(pointer: UnsafeMutablePointer<WuiGesture>) {
        let gesture = pointer.pointee
        switch gesture.kind {
        case WuiGestureKind_Tap:
            self = .tap(TapGestureDescriptor(count: gesture.tap.count))
        case WuiGestureKind_LongPress:
            self = .longPress(LongPressGestureDescriptor(duration: gesture.long_press.duration))
        case WuiGestureKind_Drag:
            self = .drag(DragGestureDescriptor(minDistance: gesture.drag.min_distance))
        case WuiGestureKind_Magnification:
            self = .magnification(
                MagnificationGestureDescriptor(initialScale: gesture.magnification.initial_scale)
            )
        case WuiGestureKind_Rotation:
            self = .rotation(RotationGestureDescriptor(initialAngle: gesture.rotation.initial_angle))
        case WuiGestureKind_Then:
            let firstDescriptor: GestureDescriptor? = gesture.first.map { GestureDescriptor(pointer: $0) }
            let secondDescriptor: GestureDescriptor? = gesture.then.map { GestureDescriptor(pointer: $0) }
            switch (firstDescriptor, secondDescriptor) {
            case let (.some(first), .some(second)):
                self = .sequence(first: first, then: second)
            case let (.some(first), .none):
                self = first
            case let (.none, .some(second)):
                self = second
            case (nil, nil):
                self = .tap(TapGestureDescriptor(count: 1))
            }
            default:
                fatalError("Unknown gesture kind: \(gesture.kind.rawValue)")
        }
       
    }

    func flattened() -> [BaseGestureDescriptor] {
        switch self {
        case .tap(let tap):
            return [.tap(tap)]
        case .longPress(let press):
            return [.longPress(press)]
        case .drag(let drag):
            return [.drag(drag)]
        case .magnification(let magnification):
            return [.magnification(magnification)]
        case .rotation:
            // Rotation events are not exposed through the current FFI payload.
            // Ignore the descriptor until a rotation event is available.
            return []
        case .sequence(let first, let then):
            return first.flattened() + then.flattened()
        }
    }
}

// MARK: - Helpers

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
