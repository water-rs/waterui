use kurbo::{Affine, Circle, Ellipse, Rect, RoundedRect, RoundedRectRadii};
use objc2_quartz_core::CACornerMask;

use cherenkov::{ContinuousRect, ShapeData};

use super::{LayerClip, LayerPlanes};
use crate::render::planes::Compositor;

fn rect() -> Rect {
    Rect::new(10.0, 20.0, 110.0, 70.0)
}

#[test]
fn motionless_placement_needs_no_animation_transaction() {
    let mut state = super::MotionState::default();
    assert!(!state.update(&[]));
    for _ in 0..3 {
        state.placed(false);
        assert!(!state.update(&[]));
    }
}

#[test]
fn a_rebuilt_motion_tree_reinstalls_unchanged_tracks() {
    let mut state = super::MotionState::default();
    assert!(!state.update(&[]));
    state.placed(true);
    assert!(state.update(&[]));
    state.placed(false);
    assert!(!state.update(&[]));
}

#[test]
fn a_rect_is_a_layer_clip() {
    assert_eq!(
        LayerClip::of(&ShapeData::Rect(rect())),
        Some(LayerClip {
            rect: rect(),
            radius: 0.0,
            corners: CACornerMask::empty(),
            continuous: false,
        })
    );
}

/// A layer rounds the corners it masks with one radius: square corners mix
/// with rounded ones, two radii do not.
#[test]
fn rounded_corners_share_one_radius_or_are_square() {
    let mixed = RoundedRect::from_rect(rect(), RoundedRectRadii::new(8.0, 0.0, 8.0, 0.0));
    assert_eq!(
        LayerClip::of(&ShapeData::RoundedRect(mixed)),
        Some(LayerClip {
            rect: rect(),
            radius: 8.0,
            corners: CACornerMask::LayerMinXMinYCorner | CACornerMask::LayerMaxXMaxYCorner,
            continuous: false,
        })
    );
    let two = RoundedRect::from_rect(rect(), RoundedRectRadii::new(8.0, 4.0, 8.0, 4.0));
    assert_eq!(LayerClip::of(&ShapeData::RoundedRect(two)), None);
}

#[test]
fn a_radius_beyond_half_the_short_side_is_not_a_layer_clip() {
    let fits = RoundedRect::from_rect(rect(), 25.0);
    assert!(LayerClip::of(&ShapeData::RoundedRect(fits)).is_some());
    let over = ContinuousRect {
        rect: rect(),
        radii: RoundedRectRadii::from_single_radius(26.0),
        smoothing: 0.0,
    };
    assert_eq!(LayerClip::of(&ShapeData::Continuous(over)), None);
}

#[test]
fn circles_and_round_ellipses_are_layer_clips_ovals_are_not() {
    let circle = LayerClip::of(&ShapeData::Circle(Circle::new((50.0, 50.0), 10.0)))
        .expect("a circle is a fully rounded square");
    assert_eq!(circle.rect, Rect::new(40.0, 40.0, 60.0, 60.0));
    assert!((circle.radius - 10.0).abs() < f64::EPSILON);
    let round = Ellipse::new((50.0, 50.0), (10.0, 10.0), 0.7);
    let round = LayerClip::of(&ShapeData::Ellipse(round)).expect("a rotated circle");
    assert!((round.radius - 10.0).abs() < 1e-9);
    let oval = Ellipse::new((50.0, 50.0), (10.0, 6.0), 0.0);
    assert_eq!(LayerClip::of(&ShapeData::Ellipse(oval)), None);
}

/// Continuous corners are expressible at the system's own smoothing
/// (`cornerCurve = continuous`) or at zero (circular), nowhere else.
#[test]
fn continuous_corners_need_the_system_smoothing() {
    let at = |smoothing| {
        LayerClip::of(&ShapeData::Continuous(ContinuousRect {
            rect: rect(),
            radii: RoundedRectRadii::from_single_radius(12.0),
            smoothing,
        }))
    };
    assert!(at(ContinuousRect::DEFAULT_SMOOTHING).is_some_and(|c| c.continuous));
    assert!(at(0.0).is_some_and(|c| !c.continuous));
    assert_eq!(at(0.3), None);
}

#[test]
fn paths_are_not_layer_clips() {
    let path = ShapeData::Path {
        elements: vec![
            kurbo::PathEl::MoveTo((0.0, 0.0).into()),
            kurbo::PathEl::ClosePath,
        ]
        .into(),
        rule: cherenkov::FillRule::NonZero,
    };
    assert_eq!(LayerClip::of(&path), None);
    assert!(!LayerPlanes::expresses_clip(&path));
}

#[test]
fn every_finite_affine_is_expressible() {
    assert!(LayerPlanes::expresses_transform(
        Affine::rotate(0.4) * Affine::skew(0.2, 0.0)
    ));
    assert!(!LayerPlanes::expresses_transform(Affine::scale(f64::NAN)));
}

#[cfg(not(target_os = "macos"))]
mod hosted {
    use kurbo::{Affine, Rect, Size, Vec2};
    use objc2::rc::Retained;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_quartz_core::{CALayer, CAMetalLayer};
    use rustc_hash::FxHashMap;

    use cherenkov::{LayerId, ShapeData};

    use super::super::{HostedNode, LayerScene, Transaction, anchored, host};
    use crate::render::planes::{Level, Placement, Source};

    const ROOT: LayerId = LayerId::new(0);
    const PARENT: LayerId = LayerId::new(1);
    const WEB: LayerId = LayerId::new(2);
    const OTHER: LayerId = LayerId::new(3);

    /// The scene's window handle is held, never read: a headless scene
    /// has none.
    struct NoWindow;

    impl wgpu::rwh::HasWindowHandle for NoWindow {
        fn window_handle(&self) -> Result<wgpu::rwh::WindowHandle<'_>, wgpu::rwh::HandleError> {
            Err(wgpu::rwh::HandleError::NotSupported)
        }
    }

    impl wgpu::rwh::HasDisplayHandle for NoWindow {
        fn display_handle(&self) -> Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
            Err(wgpu::rwh::HandleError::NotSupported)
        }
    }

    /// A scene with `parts` engine parts and no window: Core Animation
    /// layers in memory, never attached to a display.
    fn scene(parts: usize) -> LayerScene {
        LayerScene {
            _window: Box::new(NoWindow),
            root: anchored(),
            parts: (0..parts).map(|_| CAMetalLayer::new()).collect(),
            planes: Vec::new(),
            displays: FxHashMap::default(),
            retired: Vec::new(),
            motions: FxHashMap::default(),
            rasters: FxHashMap::default(),
            retired_rasters: Vec::new(),
            hosted: FxHashMap::default(),
        }
    }

    fn ptr(layer: &CALayer) -> *const CALayer {
        std::ptr::from_ref(layer)
    }

    fn superlayer(layer: &CALayer) -> Option<*const CALayer> {
        layer.superlayer().map(|parent| Retained::as_ptr(&parent))
    }

    fn holder(nodes: &FxHashMap<LayerId, HostedNode>, layer: LayerId) -> *const CALayer {
        Retained::as_ptr(&nodes[&layer].holder)
    }

    const EXTENT: Size = Size::new(300.5, 200.0);

    /// The host's layer sits at the holder's origin at its extent; the
    /// bounds origin — the host's own scroll — stays the host's.
    #[test]
    fn a_hosted_layer_is_placed_in_its_holder_at_its_extent() {
        let _tx = Transaction::begin();
        let web = CALayer::new();
        web.setBounds(CGRect::new(CGPoint::new(5.0, 7.0), CGSize::new(1.0, 1.0)));
        web.setPosition(CGPoint::new(90.0, 40.0));
        let mut nodes = FxHashMap::default();
        host(&mut nodes, vec![(WEB, web.clone(), EXTENT)]);
        assert_eq!(superlayer(&web), Some(holder(&nodes, WEB)));
        let bounds = web.bounds();
        assert_eq!(
            (
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height
            ),
            (5.0, 7.0, 300.5, 200.0)
        );
        let (position, anchor) = (web.position(), web.anchorPoint());
        assert_eq!(
            (position.x, position.y, anchor.x, anchor.y),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    /// A new extent, or another object rebound on the same layer, keeps
    /// the holder — the plane's leaf — and swaps or resizes inside it.
    #[test]
    fn a_rebinding_keeps_the_holder() {
        let _tx = Transaction::begin();
        let (web, next) = (CALayer::new(), CALayer::new());
        let mut nodes = FxHashMap::default();
        host(&mut nodes, vec![(WEB, web.clone(), EXTENT)]);
        let first = holder(&nodes, WEB);
        host(
            &mut nodes,
            vec![(WEB, web.clone(), Size::new(640.0, 480.0))],
        );
        assert_eq!(holder(&nodes, WEB), first);
        assert!((web.bounds().size.width - 640.0).abs() < f64::EPSILON);
        host(&mut nodes, vec![(WEB, next.clone(), EXTENT)]);
        assert_eq!(holder(&nodes, WEB), first);
        assert_eq!(superlayer(&web), None, "the replaced object leaves");
        assert_eq!(superlayer(&next), Some(first));
    }

    /// An unbound object leaves the engine's tree — unless it moved to
    /// another holder first, which then owns it.
    #[test]
    fn an_unbound_object_leaves_unless_it_moved() {
        let _tx = Transaction::begin();
        let web = CALayer::new();
        let (mut here, mut there) = (FxHashMap::default(), FxHashMap::default());
        host(&mut here, vec![(WEB, web.clone(), EXTENT)]);
        host(&mut there, vec![(OTHER, web.clone(), EXTENT)]);
        assert_eq!(superlayer(&web), Some(holder(&there, OTHER)));
        host(&mut here, Vec::new());
        assert!(here.is_empty());
        assert_eq!(superlayer(&web), Some(holder(&there, OTHER)));
        host(&mut there, Vec::new());
        assert_eq!(superlayer(&web), None);
    }

    fn placement(offset: Vec2) -> Placement {
        let level = |layer, transform, clip| Level {
            layer,
            transform,
            clip,
            scroll: Vec2::ZERO,
        };
        Placement {
            layer: WEB,
            source: Source::Hosted,
            size: (301, 200),
            raster: Affine::IDENTITY,
            opacity: 1.0,
            path: vec![
                level(ROOT, Affine::IDENTITY, None),
                level(
                    PARENT,
                    Affine::translate(offset),
                    Some(ShapeData::Rect(Rect::new(0.0, 0.0, 200.0, 100.0))),
                ),
                level(WEB, Affine::IDENTITY, None),
            ],
        }
    }

    /// The hosted plane sits between the part painted below it and the
    /// part painted above it, inside its path's level nodes, and a move
    /// on its path updates those nodes in place.
    #[test]
    fn a_hosted_plane_sits_between_its_parts_and_moves_in_place() {
        let _tx = Transaction::begin();
        let web = CALayer::new();
        let mut scene = scene(2);
        host(&mut scene.hosted, vec![(WEB, web.clone(), EXTENT)]);
        scene.place(&[placement(Vec2::new(10.0, 20.0))], (800, 600), 2.0, 2);
        // SAFETY: the array is read at once, while nothing mutates the
        // root's sublayers.
        let sublayers = unsafe { scene.root.sublayers() };
        let order: Vec<_> = sublayers
            .expect("the stack")
            .iter()
            .map(|layer| Retained::as_ptr(&layer))
            .collect();
        let top = Retained::as_ptr(&scene.planes[0].top);
        let parts: Vec<_> = scene
            .parts
            .iter()
            .map(|part| Retained::as_ptr(part).cast::<CALayer>())
            .collect();
        assert_eq!(order, [parts[0], top, parts[1]]);
        let leaf = &scene.planes[0].levels[2];
        assert_eq!(
            superlayer(&scene.hosted[&WEB].holder),
            Some(ptr(leaf.inner()))
        );
        assert_eq!(superlayer(&web), Some(holder(&scene.hosted, WEB)));
        let clip = scene.planes[0].levels[1].clip.clone().expect("clip node");
        assert!(clip.masksToBounds());
        let node = scene.planes[0].levels[1].node.clone();
        let holder_bounds = scene.hosted[&WEB].holder.bounds();
        assert!((holder_bounds.size.width - 300.5).abs() < f64::EPSILON);

        scene.place(&[placement(Vec2::new(64.0, 8.0))], (800, 600), 2.0, 2);
        assert_eq!(
            Retained::as_ptr(&scene.planes[0].top),
            top,
            "the plane stays"
        );
        assert_eq!(
            Retained::as_ptr(&scene.planes[0].levels[1].node),
            Retained::as_ptr(&node)
        );
        let moved = node.affineTransform();
        assert_eq!((moved.tx, moved.ty), (64.0, 8.0));
        assert_eq!(superlayer(&web), Some(holder(&scene.hosted, WEB)));
    }
}
