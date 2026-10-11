// Twin of examples/gradient: animated mesh, GPU shader mesh, shape fills,
// linear/radial/mesh gradients, and HDR variants.
//
// The two animated sections cannot pixel-match a live capture (the WaterUI
// side animates continuously, so its phase is arbitrary at capture time).
// The twin reproduces them semantically — same palette, same size — so
// layout parity is checked and the diff budget absorbs animation phase.
// `computeAnimatedColors` is a direct port of the example's Rust function
// so the t=0 state is identical.
//
// Every gradient color in the example is a `ResolvedColor` — linear sRGB
// components — so the palettes go through `linearSrgb`. Only the overlay
// text (`Color::srgb` on the Rust side) stays gamma-encoded `srgb`.

import SwiftUI

private func srgb(_ r: Double, _ g: Double, _ b: Double) -> Color {
    Color(.sRGB, red: r, green: g, blue: b, opacity: 1)
}

struct GradientTwin: View {
    var body: some View {
        ScrollView {
            VStack(spacing: 10) {
                Text("WaterUI Gradient Examples").font(.system(size: 28))
                Text("GPU-rendered gradients with animation support")
                wuiDivider()
                animatedBackgroundSection
                wuiDivider()
                gpuMeshSection
                wuiDivider()
                shapeFillSection
                wuiDivider()
                linearSection
                wuiDivider()
                radialSection
                wuiDivider()
                VStack(spacing: 10) {
                    meshSection
                    wuiDivider()
                    hdrSection
                }
            }
            .padding(16)
        }
    }

    // Port of compute_animated_colors(t) from examples/gradient.
    private func animatedColors(_ time: Double) -> [Color] {
        let base: [[Double]] = [
            [0.1, 0.1, 0.3], [0.2, 0.1, 0.4], [0.3, 0.1, 0.3],
            [0.1, 0.2, 0.4], [0.2, 0.3, 0.5], [0.3, 0.2, 0.4],
            [0.05, 0.15, 0.3], [0.1, 0.2, 0.35], [0.15, 0.1, 0.25],
        ]
        return base.indices.map { i in
            let phase = Double(i) * 0.7
            let x = Double(i % 3)
            let y = Double(i / 3)
            let w1 = sin(time * 0.5 + phase) * 0.15
            let w2 = sin(time * 0.3 + x * 0.5) * 0.1
            let w3 = cos(time * 0.7 + y * 0.8) * 0.08
            return linearSrgb(
                min(max(base[i][0] + w1 + w2 * 0.5, 0), 1),
                min(max(base[i][1] + w2 + w3 * 0.5, 0), 1),
                min(max(base[i][2] + w3 + w1 * 0.5, 0), 1)
            )
        }
    }

    // The example seeds the binding with compute_animated_colors(0.0), so t=0
    // is the one phase both sides can agree on; a live TimelineView would put
    // the reference on an unrelated wall clock and make the ref itself
    // non-deterministic run to run.
    private var animatedBackgroundSection: some View {
        VStack(spacing: 12) {
            Text("Animated Mesh Gradient").font(.system(size: 20))
            Text("Automatic time-based fluid animation")
            MeshGradient(
                width: 3, height: 3,
                points: [
                    [0, 0], [0.5, 0], [1, 0],
                    [0, 0.5], [0.5, 0.5], [1, 0.5],
                    [0, 1], [0.5, 1], [1, 1],
                ],
                colors: animatedColors(0)
            )
            .frame(width: 300, height: 200)
            .overlay(
                VStack(spacing: 10) {
                    Text("Fluid Background").font(.system(size: 24))
                        .foregroundStyle(srgb(1, 1, 1))
                    Text("Colors flow over time").foregroundStyle(srgb(200.0 / 255, 200.0 / 255, 1))
                }
                .padding(14)
            )
            .frame(width: 300, height: 200)
        }
        .padding(14)
    }

    // aqua_bloom palette (4x4), approximating the GPU shader gradient with a
    // static 4x4 MeshGradient.
    private static let aquaBloom: [Color] = [
        linearSrgb(0.60, 0.92, 0.98), linearSrgb(0.70, 0.90, 0.98), linearSrgb(0.78, 0.84, 0.96), linearSrgb(0.84, 0.92, 0.98),
        linearSrgb(0.38, 0.72, 0.92), linearSrgb(0.46, 0.64, 0.90), linearSrgb(0.82, 0.64, 0.92), linearSrgb(0.90, 0.74, 0.92),
        linearSrgb(0.30, 0.56, 0.86), linearSrgb(0.42, 0.52, 0.86), linearSrgb(0.78, 0.56, 0.86), linearSrgb(0.94, 0.70, 0.88),
        linearSrgb(0.22, 0.44, 0.78), linearSrgb(0.36, 0.46, 0.80), linearSrgb(0.62, 0.54, 0.80), linearSrgb(0.86, 0.66, 0.84),
    ]

    private var gpuMeshSection: some View {
        VStack(spacing: 12) {
            Text("GPU Animated Mesh Gradient").font(.system(size: 20))
            Text("Speed + palette configured at creation time")
            MeshGradient(
                width: 4, height: 4,
                points: Self.grid4,
                colors: Self.aquaBloom
            )
            .frame(width: 300, height: 200)
            .overlay(
                VStack(spacing: 10) {
                    Text("Mesh Gradient").font(.system(size: 24))
                        .foregroundStyle(srgb(1, 1, 1))
                    Text("No per-frame CPU updates")
                        .foregroundStyle(srgb(200.0 / 255, 220.0 / 255, 1))
                }
                .padding(14)
            )
        }
        .padding(14)
    }

    private static let grid4: [SIMD2<Float>] = [
        [0, 0], [1.0 / 3, 0], [2.0 / 3, 0], [1, 0],
        [0, 1.0 / 3], [1.0 / 3, 1.0 / 3], [2.0 / 3, 1.0 / 3], [1, 1.0 / 3],
        [0, 2.0 / 3], [1.0 / 3, 2.0 / 3], [2.0 / 3, 2.0 / 3], [1, 2.0 / 3],
        [0, 1], [1.0 / 3, 1], [2.0 / 3, 1], [1, 1],
    ]

    private func linear(_ stops: [(Double, Color)], from: UnitPoint, to: UnitPoint) -> LinearGradient {
        LinearGradient(stops: stops.map { .init(color: $0.1, location: $0.0) }, startPoint: from, endPoint: to)
    }

    private var shapeFillSection: some View {
        VStack(spacing: 10) {
            Text("Shape + Gradient Fill").font(.system(size: 20))
            Text("Gradients clipped to shapes on the GPU")
            HStack(spacing: 16) {
                VStack(spacing: 10) {
                    linear([(0, linearSrgb(1, 0.3, 0.5)), (1, linearSrgb(0.3, 0.5, 1))], from: .topLeading, to: .bottomTrailing)
                        .frame(width: 100, height: 100)
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                    Text("Linear + Rounded")
                }
                VStack(spacing: 10) {
                    RadialGradient(
                        stops: [
                            .init(color: linearSrgb(1, 1, 0.8), location: 0),
                            .init(color: linearSrgb(1, 0.6, 0.2), location: 0.5 / 0.7),
                            .init(color: linearSrgb(0.6, 0.2, 0.1), location: 1),
                        ],
                        center: .center, startRadius: 0, endRadius: 70
                    )
                    .frame(width: 100, height: 100)
                    .clipShape(Circle())
                    Text("Radial + Circle")
                }
                VStack(spacing: 10) {
                    MeshGradient(
                        width: 2, height: 2,
                        points: [[0, 0], [1, 0], [0, 1], [1, 1]],
                        colors: [linearSrgb(0, 0.8, 0.4), linearSrgb(0, 0.4, 0.8), linearSrgb(0.8, 0.4, 0), linearSrgb(0.8, 0, 0.4)]
                    )
                    .frame(width: 100, height: 100)
                    .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
                    Text("Mesh + Rounded")
                }
            }
        }
        .padding(14)
    }

    private var linearSection: some View {
        VStack(spacing: 10) {
            Text("Linear Gradients").font(.system(size: 20))
            Text("Gradients along a line from start to end point")
            HStack(spacing: 16) {
                VStack(spacing: 10) {
                    linear([(0, linearSrgb(1, 0, 0)), (0.5, linearSrgb(1, 1, 0)), (1, linearSrgb(0, 1, 0))], from: .leading, to: .trailing)
                        .frame(width: 120, height: 80)
                    Text("Horizontal")
                }
                VStack(spacing: 10) {
                    linear([(0, linearSrgb(0, 0.5, 1)), (1, linearSrgb(0, 0, 0.5))], from: .top, to: .bottom)
                        .frame(width: 120, height: 80)
                    Text("Vertical")
                }
                VStack(spacing: 10) {
                    linear([(0, linearSrgb(1, 0, 1)), (1, linearSrgb(0, 1, 1))], from: .topLeading, to: .bottomTrailing)
                        .frame(width: 120, height: 80)
                    Text("Diagonal")
                }
            }
        }
        .padding(14)
    }

    private var radialSection: some View {
        VStack(spacing: 10) {
            Text("Radial Gradients").font(.system(size: 20))
            Text("Gradients expanding outward from a center point")
            HStack(spacing: 16) {
                VStack(spacing: 10) {
                    RadialGradient(
                        stops: [
                            .init(color: linearSrgb(1, 1, 1), location: 0),
                            .init(color: linearSrgb(1, 0.8, 0), location: 0.5 / 0.7),
                            .init(color: linearSrgb(1, 0.3, 0), location: 1),
                        ],
                        center: .center, startRadius: 0, endRadius: 0.7 * 120
                    )
                    .frame(width: 120, height: 120)
                    Text("Centered")
                }
                VStack(spacing: 10) {
                    RadialGradient(
                        stops: [
                            .init(color: linearSrgb(1, 1, 1), location: 0),
                            .init(color: linearSrgb(0.2, 0.4, 1), location: 1),
                        ],
                        center: UnitPoint(x: 0.3, y: 0.3), startRadius: 0, endRadius: 0.8 * 120
                    )
                    .frame(width: 120, height: 120)
                    Text("Off-center")
                }
            }
        }
        .padding(14)
    }

    private var meshSection: some View {
        VStack(spacing: 10) {
            Text("Static Mesh Gradients").font(.system(size: 20))
            Text("Gradients with per-vertex colors interpolated across a grid")
            HStack(spacing: 16) {
                VStack(spacing: 10) {
                    MeshGradient(
                        width: 2, height: 2,
                        points: [[0, 0], [1, 0], [0, 1], [1, 1]],
                        colors: [linearSrgb(1, 0, 0), linearSrgb(0, 1, 0), linearSrgb(0, 0, 1), linearSrgb(1, 1, 0)]
                    )
                    .frame(width: 120, height: 120)
                    Text("2x2 Corners")
                }
                VStack(spacing: 10) {
                    MeshGradient(
                        width: 3, height: 3,
                        points: [
                            [0, 0], [0.5, 0], [1, 0],
                            [0, 0.5], [0.5, 0.5], [1, 0.5],
                            [0, 1], [0.5, 1], [1, 1],
                        ],
                        colors: [
                            linearSrgb(0.2, 0.2, 0.4), linearSrgb(0.3, 0.3, 0.5), linearSrgb(0.2, 0.2, 0.4),
                            linearSrgb(0.3, 0.3, 0.5), linearSrgb(1, 1, 1), linearSrgb(0.3, 0.3, 0.5),
                            linearSrgb(0.2, 0.2, 0.4), linearSrgb(0.3, 0.3, 0.5), linearSrgb(0.2, 0.2, 0.4),
                        ]
                    )
                    .frame(width: 120, height: 120)
                    Text("3x3 Highlight")
                }
            }
        }
        .padding(14)
    }

    private var hdrSection: some View {
        VStack(spacing: 12) {
            Text("HDR Gradients").font(.system(size: 20))
            Text("Extended brightness beyond SDR (requires HDR display)")
            HStack(spacing: 16) {
                VStack(spacing: 10) {
                    RadialGradient(
                        stops: [.init(color: linearSrgb(1, 1, 1), location: 0), .init(color: linearSrgb(0.1, 0.1, 0.2), location: 1)],
                        center: .center, startRadius: 0, endRadius: 0.7 * 120
                    )
                    .frame(width: 120, height: 120)
                    Text("SDR White")
                }
                VStack(spacing: 10) {
                    RadialGradient(
                        stops: [.init(color: linearSrgb(1, 1, 1), location: 0), .init(color: linearSrgb(0.1, 0.1, 0.2), location: 1)],
                        center: .center, startRadius: 0, endRadius: 0.7 * 120
                    )
                    .frame(width: 120, height: 120)
                    Text("HDR 1.5x")
                }
                VStack(spacing: 10) {
                    RadialGradient(
                        stops: [.init(color: linearSrgb(1, 1, 1), location: 0), .init(color: linearSrgb(0.1, 0.1, 0.2), location: 1)],
                        center: .center, startRadius: 0, endRadius: 0.7 * 120
                    )
                    .frame(width: 120, height: 120)
                    Text("HDR 2x")
                }
            }
            HStack(spacing: 16) {
                VStack(spacing: 10) {
                    linear([(0, linearSrgb(1, 0.3, 0.3)), (1, linearSrgb(0.3, 0, 0))], from: .topLeading, to: .bottomTrailing)
                        .frame(width: 120, height: 80)
                    Text("HDR Red")
                }
                VStack(spacing: 10) {
                    linear([(0, linearSrgb(0.3, 1, 0.3)), (1, linearSrgb(0, 0.3, 0))], from: .topLeading, to: .bottomTrailing)
                        .frame(width: 120, height: 80)
                    Text("HDR Green")
                }
                VStack(spacing: 10) {
                    linear([(0, linearSrgb(0.3, 0.3, 1)), (1, linearSrgb(0, 0, 0.3))], from: .topLeading, to: .bottomTrailing)
                        .frame(width: 120, height: 80)
                    Text("HDR Blue")
                }
            }
        }
        .padding(14)
    }
}
