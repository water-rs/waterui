//! Scene serde round-trip and path/shape helper checks.

const NONZERO: std::num::NonZeroU32 = std::num::NonZeroU32::MIN;
use cherenkov_scene::{
    BackdropGroup, BlendMode, Color, ColorSpace, Feature, FillRule, Glyph, GlyphRun, GradientStop,
    Item, LinearGradient, Paint, ResourceHash, Sampling, Scene, Shape, StrokeStyle,
};
use kurbo::{Affine, BezPath, Point, Rect};

#[test]
fn color_roundtrip() {
    let c = Color::srgb(1.0, 0.5, 0.25).with_alpha(0.75);
    let json = serde_json::to_string(&c).unwrap();
    let back: Color = serde_json::from_str(&json).unwrap();
    assert_eq!(c, back);
    assert!(!c.is_hdr());
    let hdr = Color::new(ColorSpace::LinearP3, [2.0, 0.0, 0.0, 1.0]);
    assert!(hdr.is_hdr());
    assert!(hdr.is_wide_gamut());
}

#[test]
fn hash_roundtrip() {
    let h = ResourceHash::of(b"hello world");
    let s = h.to_string();
    assert_eq!(s.len(), 64);
    let back: ResourceHash = s.parse().unwrap();
    assert_eq!(h, back);
    let json = serde_json::to_string(&h).unwrap();
    assert_eq!(json, format!("\"{s}\""));
    let back: ResourceHash = serde_json::from_str(&json).unwrap();
    assert_eq!(h, back);
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the fixture scene is one long literal"
)]
fn scene_save_load() {
    let dir = std::env::temp_dir().join(format!("cherenkov-scene-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let mut path = BezPath::new();
    path.move_to((10.0, 10.0));
    path.curve_to((30.0, 40.0), (60.0, 40.0), (90.0, 10.0));
    path.close_path();

    let font = ResourceHash::of(b"fake font");
    let scene = Scene::builder(128, 96)
        .clear(Color::srgb(0.0, 0.0, 0.0))
        .fill_rule(
            Shape::Path { path },
            FillRule::EvenOdd,
            Paint::Linear(LinearGradient {
                start: Point::new(0.0, 0.0),
                end: Point::new(128.0, 96.0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: Color::srgb(1.0, 0.0, 0.0),
                    },
                    GradientStop {
                        offset: 1.0,
                        color: Color::srgb(0.0, 0.0, 1.0),
                    },
                ],
                extend: cherenkov_scene::Extend::Repeat,
                interpolation: ColorSpace::Srgb,
            }),
        )
        .stroke(
            Shape::circle(64.0, 48.0, 20.0),
            StrokeStyle {
                width: 2.0,
                dash_pattern: vec![3.0, 1.0],
                ..StrokeStyle::default()
            },
            Paint::Solid(Color::new(ColorSpace::DisplayP3, [0.0, 1.0, 0.0, 1.0])),
        )
        .glyphs(GlyphRun {
            stroke: None,
            font,
            font_index: 0,
            size: 24.0,
            normalized_coords: vec![],
            glyphs: vec![Glyph {
                id: 36,
                x: 0.0,
                y: 0.0,
                transform: None,
            }],
            paint: Paint::Solid(Color::srgb(0.0, 0.0, 0.0)),
        })
        .image(
            ResourceHash::of(b"fake image"),
            Rect::new(0.0, 0.0, 32.0, 32.0),
            Sampling::Nearest,
        )
        .layer(|l| {
            l.blend(BlendMode::Multiply)
                .opacity(0.5)
                .clip(Shape::rounded_rect(4.0, 4.0, 60.0, 60.0, 8.0))
                .transform(Affine::rotate(0.2))
                .shadow(
                    Shape::rect(10.0, 10.0, 50.0, 30.0),
                    4.0,
                    [2.0, 3.0],
                    Color::srgb(0.0, 0.0, 0.0).with_alpha(0.5),
                );
        })
        .build();

    for feat in [
        Feature::Fill,
        Feature::EvenOdd,
        Feature::LinearGradient,
        Feature::Stroke,
        Feature::StrokeDash,
        Feature::Path,
        Feature::Glyphs,
        Feature::Image,
        Feature::Clip,
        Feature::Opacity,
        Feature::Shadow,
        Feature::Blend(BlendMode::Multiply),
        Feature::WideGamut,
    ] {
        assert!(scene.features.contains(&feat), "missing {feat:?}");
    }

    scene.save(&dir).unwrap();
    let blob = Scene::store_resource(&dir, b"fake font").unwrap();
    assert_eq!(blob, font);
    let loaded = Scene::load(&dir).unwrap();
    assert_eq!(scene, loaded);
    assert_eq!(Scene::resource(&dir, font).unwrap(), b"fake font");
    assert!(matches!(
        Scene::resource(&dir, ResourceHash::of(b"absent")),
        Err(cherenkov_scene::SceneError::MissingResource(_))
    ));

    let refs = scene.resource_refs();
    assert!(refs.contains(&font));
    assert!(refs.contains(&ResourceHash::of(b"fake image")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn present_headroom_roundtrip() {
    let dir = std::env::temp_dir().join(format!(
        "cherenkov-scene-headroom-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    // The default stays out of the JSON, so existing scenes are
    // byte-identical.
    let scene = Scene::builder(8, 8).build();
    let json = serde_json::to_string_pretty(&scene).unwrap();
    assert!(!json.contains("present_headroom"), "{json}");
    scene.save(&dir).unwrap();
    assert_eq!(Scene::load(&dir).unwrap(), scene);

    // A declared headroom serializes and loads back.
    let scene = Scene::builder(8, 8).present_headroom(4.0).build();
    let json = serde_json::to_string_pretty(&scene).unwrap();
    assert!(json.contains("\"present_headroom\": 4.0"), "{json}");
    scene.save(&dir).unwrap();
    assert_eq!(Scene::load(&dir).unwrap(), scene);

    // A file carrying the field parses without a `Scene::load` too.
    let parsed: Scene = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, scene);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn item_untagged() {
    let layer_json = r#"{"items":[]}"#;
    let layer: cherenkov_scene::Layer = serde_json::from_str(layer_json).unwrap();
    assert_eq!(layer.items, []);
    let item_layer: Item = serde_json::from_str(r#"{"layer":{"items":[]}}"#).unwrap();
    assert!(matches!(item_layer, Item::Layer(_)));
    let item_draw: Item = serde_json::from_str(
        r#"{"draw":{"image":{"image":"0000000000000000000000000000000000000000000000000000000000000000",
             "dst":{"x0":0.0,"y0":0.0,"x1":1.0,"y1":1.0},"sampling":"nearest"}}}"#,
    )
    .unwrap();
    assert!(matches!(
        item_draw,
        Item::Draw(cherenkov_scene::Draw::Image { .. })
    ));
}

#[test]
fn continuous_rect_path_valid() {
    let shape = Shape::Continuous(cherenkov_scene::ContinuousRect {
        rect: Rect::new(0.0, 0.0, 100.0, 80.0),
        corner_radius: 20.0,
        smoothing: 0.6,
    });
    let path = shape.to_path();
    let mut has_close = false;
    for el in path.elements() {
        if let Some(p) = el.end_point() {
            assert!(p.x.is_finite() && p.y.is_finite());
        }
        if matches!(el, kurbo::PathEl::ClosePath) {
            has_close = true;
        }
    }
    assert!(has_close);
    let bb = shape.bounding_box();
    assert!((bb.x1 - bb.x0 - 100.0).abs() < 1e-6);
    assert!((bb.y1 - bb.y0 - 80.0).abs() < 1e-6);
}

#[test]
fn smoothing_zero_matches_circular_corner() {
    // smoothing = 0 -> exponent 2 -> a circle quarter approximating the corner.
    let shape = Shape::Continuous(cherenkov_scene::ContinuousRect {
        rect: Rect::new(0.0, 0.0, 40.0, 40.0),
        corner_radius: 10.0,
        smoothing: 0.0,
    });
    let path = shape.to_path();
    assert!(path.elements().len() > 10);
}

#[test]
fn image_encodings_roundtrip() {
    use cherenkov_scene::{ImageColorSpace, ImageEncoding, ImagePaint};

    let png_hash = ResourceHash::of(b"fake-png");
    let raw_hash = ResourceHash::of(b"fake-f16");
    let mut scene = Scene::builder(8, 8);
    scene.root().image_encoded(
        png_hash,
        ImageEncoding::Png {
            color_space: ImageColorSpace::DisplayP3,
        },
        Rect::new(0.0, 0.0, 8.0, 8.0),
        Sampling::Bilinear,
    );
    scene.root().fill(
        Shape::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
        Paint::Image(ImagePaint {
            image: raw_hash,
            encoding: ImageEncoding::Rgba16F {
                width: 2,
                height: 2,
                color_space: ImageColorSpace::LinearP3,
            },
            transform: Affine::IDENTITY,
            extend_x: cherenkov_scene::Extend::Pad,
            extend_y: cherenkov_scene::Extend::Pad,
            sampling: Sampling::Nearest,
        }),
    );
    let scene = scene.build();
    assert!(scene.features.contains(&Feature::ImageF16));
    assert!(
        scene
            .features
            .contains(&Feature::ImageColorSpace(ImageColorSpace::DisplayP3))
    );
    assert!(
        scene
            .features
            .contains(&Feature::ImageColorSpace(ImageColorSpace::LinearP3))
    );

    let json = serde_json::to_string(&scene).unwrap();
    let back: Scene = serde_json::from_str(&json).unwrap();
    assert_eq!(scene, back);

    // A default (sRGB PNG) encoding is omitted from the JSON, so existing
    // scene files keep serializing byte-identically.
    let mut srgb = Scene::builder(8, 8);
    srgb.root()
        .image(png_hash, Rect::new(0.0, 0.0, 8.0, 8.0), Sampling::Bilinear);
    let srgb = srgb.build();
    assert!(!serde_json::to_string(&srgb).unwrap().contains("encoding"));

    // Encoding/colour-space combinations that cannot decode are rejected.
    for bad in [
        ImageEncoding::Png {
            color_space: ImageColorSpace::LinearP3,
        },
        ImageEncoding::Rgba16F {
            width: 0,
            height: 1,
            color_space: ImageColorSpace::LinearP3,
        },
        ImageEncoding::Rgba16F {
            width: 1,
            height: 1,
            color_space: ImageColorSpace::DisplayP3,
        },
    ] {
        assert!(bad.validate().is_err());
    }
    assert!(ImageEncoding::default().validate().is_ok());
}

#[test]
fn backdrop_effect_specs_roundtrip() {
    use cherenkov_scene::{BackdropEffectSpec, BackdropFilter};
    let specs = [
        BackdropEffectSpec::ColorMatrix {
            matrix: [
                1.1, 0.0, 0.0, 0.05, 0.0, 0.95, 0.0, 0.02, 0.0, 0.0, 0.8, 0.0,
            ],
        },
        BackdropEffectSpec::Refraction {
            depth: 12.0,
            strength: 6.0,
        },
        BackdropEffectSpec::RimLight {
            width: 10.0,
            color: [1.0, 0.9, 0.7, 1.0],
            gain: 4.0,
        },
    ];
    let dir = std::env::temp_dir().join(format!("cherenkov-scene-fx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (i, spec) in specs.iter().enumerate() {
        // The spec itself serializes tagged like BackdropFilter.
        let json = serde_json::to_string(spec).unwrap();
        let back: BackdropEffectSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(*spec, back);

        let mut b = Scene::builder(64, 64);
        b.backdrop_group(BackdropGroup::new(
            1,
            vec![BackdropFilter::GaussianBlur { sigma: 4.0 }],
            1.0,
            1,
        ));
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop(1);
            m.backdrop_effect(spec.clone());
        });
        let scene = b.build();
        assert!(scene.features.contains(&Feature::BackdropEffect));
        let scene_dir = dir.join(format!("s{i}"));
        scene.save(&scene_dir).unwrap();
        let loaded = Scene::load(&scene_dir).unwrap();
        assert_eq!(scene, loaded);
    }
}

#[test]
fn backdrop_effect_validation_cases() {
    use cherenkov_scene::{BackdropEffectSpec, SceneError};
    let dir = std::env::temp_dir().join(format!("cherenkov-scene-fxv-{}", std::process::id()));
    // A member without an effect keeps `backdrop_effect` out of the JSON.
    let mut b = Scene::builder(64, 64);
    b.backdrop_group(BackdropGroup::new(1, Vec::new(), 1.0, 1));
    b.root().layer(|m| {
        m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
        m.backdrop(1);
    });
    let scene = b.build();
    assert!(
        !serde_json::to_string(&scene)
            .unwrap()
            .contains("backdrop_effect")
    );

    // Effect without a group.
    let mut b = Scene::builder(64, 64);
    b.root().layer(|m| {
        m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
        m.backdrop_effect(BackdropEffectSpec::Refraction {
            depth: 4.0,
            strength: 4.0,
        });
    });
    let scene_dir = dir.join("no-group");
    b.build().save(&scene_dir).unwrap();
    assert!(matches!(
        Scene::load(&scene_dir),
        Err(SceneError::BackdropEffectWithoutGroup)
    ));

    // Out-of-range parameters.
    // (Non-finite matrix/parameter values cannot be expressed in JSON
    //  anyway — serde emits `null` — so these exercise only ranges.)
    for bad in [
        BackdropEffectSpec::Refraction {
            depth: 0.0,
            strength: 1.0,
        },
        BackdropEffectSpec::Refraction {
            depth: 4.0,
            strength: -1.0,
        },
        BackdropEffectSpec::RimLight {
            width: 0.0,
            color: [1.0, 1.0, 1.0, 1.0],
            gain: 1.0,
        },
        BackdropEffectSpec::RimLight {
            width: 4.0,
            color: [1.0, 1.0, 1.0, 1.0],
            gain: -1.0,
        },
    ] {
        let mut b = Scene::builder(64, 64);
        b.backdrop_group(BackdropGroup::new(1, Vec::new(), 1.0, 1));
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop(1);
            m.backdrop_effect(bad.clone());
        });
        let scene_dir = dir.join(format!("bad-{}", dir.read_dir().unwrap().count()));
        b.build().save(&scene_dir).unwrap();
        let result = Scene::load(&scene_dir);
        assert!(
            matches!(result, Err(SceneError::InvalidBackdropEffect(_))),
            "{bad:?} must be rejected, got {result:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backdrop_scale_loads_inside_its_range_only() {
    use cherenkov_scene::{BackdropFilter, BackdropGroup, SceneError};
    let dir = std::env::temp_dir().join(format!("cherenkov-scene-scale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let scene = |scale| {
        let mut b = Scene::builder(64, 64);
        b.backdrop_group(BackdropGroup::new(
            7,
            vec![BackdropFilter::GaussianBlur { sigma: 2.0 }],
            scale,
            1,
        ));
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop(7);
        });
        b.build()
    };

    let quarter = scene(0.25);
    assert!(quarter.features.contains(&Feature::BackdropScale));
    quarter.save(&dir.join("quarter")).unwrap();
    assert_eq!(Scene::load(&dir.join("quarter")).unwrap(), quarter);

    // Non-finite scales cannot be expressed in JSON (serde emits `null`),
    // so these exercise the range.
    for (i, bad) in [0.0, -0.25, 1.5].into_iter().enumerate() {
        let scene_dir = dir.join(format!("bad-{i}"));
        scene(bad).save(&scene_dir).unwrap();
        let result = Scene::load(&scene_dir);
        assert!(
            matches!(result, Err(SceneError::InvalidBackdropScale(7))),
            "scale {bad} must be rejected, got {result:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backdrop_levels_load_inside_their_range_only() {
    use cherenkov_scene::{BackdropFilter, BackdropGroup, SceneError};
    let dir = std::env::temp_dir().join(format!("cherenkov-scene-levels-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let scene = |levels| {
        let mut b = Scene::builder(64, 64);
        b.backdrop_group(BackdropGroup::new(
            7,
            vec![BackdropFilter::GaussianBlur { sigma: 2.0 }],
            0.5,
            levels,
        ));
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop(7);
        });
        b.build()
    };

    let deepest = scene(BackdropGroup::MAX_LEVELS);
    deepest.save(&dir.join("deepest")).unwrap();
    assert_eq!(Scene::load(&dir.join("deepest")).unwrap(), deepest);

    for bad in [0, 9] {
        let scene_dir = dir.join(format!("bad-{bad}"));
        scene(bad).save(&scene_dir).unwrap();
        let result = Scene::load(&scene_dir);
        assert!(
            matches!(result, Err(SceneError::InvalidBackdropLevels(7))),
            "levels {bad} must be rejected, got {result:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backdrop_levels_default_to_one_and_skip_one() {
    let dir = std::env::temp_dir().join(format!(
        "cherenkov-scene-levels-default-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let scene = |levels| {
        let mut b = Scene::builder(64, 64);
        b.backdrop_group(BackdropGroup::new(7, Vec::new(), 1.0, levels));
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop(7);
        });
        b.build()
    };
    let json = |name: &str| std::fs::read_to_string(dir.join(name).join("scene.json")).unwrap();

    let single = scene(1);
    single.save(&dir.join("single")).unwrap();
    assert!(
        !json("single").contains("\"levels\""),
        "a one-level group omits `levels`: {}",
        json("single")
    );
    let loaded = Scene::load(&dir.join("single")).unwrap();
    assert_eq!(loaded.backdrop_groups[0].levels, 1);
    assert_eq!(loaded, single);

    let pyramid = scene(3);
    pyramid.save(&dir.join("pyramid")).unwrap();
    assert!(json("pyramid").contains("\"levels\""));
    assert_eq!(Scene::load(&dir.join("pyramid")).unwrap(), pyramid);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backdrop_union_and_outer_roundtrip_and_validate() {
    use cherenkov_scene::{BackdropFilter, BackdropGroup, SceneError};
    let dir = std::env::temp_dir().join(format!("cherenkov-scene-union-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // A union group plus an `outer` member round-trips.
    let mut b = Scene::builder(64, 64);
    b.backdrop_group(BackdropGroup {
        union: Some(12.0),
        ..BackdropGroup::new(3, vec![BackdropFilter::GaussianBlur { sigma: 4.0 }], 1.0, 1)
    });
    b.backdrop_group(BackdropGroup::new(4, Vec::new(), 1.0, 1));
    let l = &mut b.root();
    l.layer(|m| {
        m.clip(Shape::rect(8.0, 8.0, 32.0, 32.0));
        m.backdrop(3);
    });
    l.layer(|m| {
        m.clip(Shape::rect(36.0, 8.0, 56.0, 32.0));
        m.backdrop(3);
        m.backdrop_outer(6.0);
    });
    l.layer(|m| {
        m.clip(Shape::rect(8.0, 40.0, 56.0, 56.0));
        m.backdrop(4);
        m.backdrop_outer(2.5);
    });
    let scene = b.build();
    assert!(scene.features.contains(&Feature::BackdropUnion));
    assert!(scene.features.contains(&Feature::BackdropOuter));
    let scene_dir = dir.join("union");
    scene.save(&scene_dir).unwrap();
    assert_eq!(Scene::load(&scene_dir).unwrap(), scene);

    // A `backdrop_outer` of 0 is the default and stays out of the JSON.
    let mut b = Scene::builder(64, 64);
    b.backdrop_group(BackdropGroup::new(4, Vec::new(), 1.0, 1));
    b.root().layer(|m| {
        m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
        m.backdrop(4);
    });
    let scene = b.build();
    assert!(
        !serde_json::to_string(&scene)
            .unwrap()
            .contains("backdrop_outer")
    );

    // Invalid values are rejected at load: a non-positive union, a
    // negative outer, and an outer on a layer sampling no group.
    let bad_union = {
        let mut b = Scene::builder(64, 64);
        b.backdrop_group(BackdropGroup {
            union: Some(0.0),
            ..BackdropGroup::new(3, Vec::new(), 1.0, 1)
        });
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop(3);
        });
        b.build()
    };
    let scene_dir = dir.join("bad-union");
    bad_union.save(&scene_dir).unwrap();
    let result = Scene::load(&scene_dir);
    assert!(
        matches!(result, Err(SceneError::InvalidBackdropUnion(3))),
        "union 0 must be rejected, got {result:?}"
    );

    let bad_outer = {
        let mut b = Scene::builder(64, 64);
        b.backdrop_group(BackdropGroup::new(4, Vec::new(), 1.0, 1));
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop(4);
            m.backdrop_outer(-1.0);
        });
        b.build()
    };
    let scene_dir = dir.join("bad-outer");
    bad_outer.save(&scene_dir).unwrap();
    let result = Scene::load(&scene_dir);
    assert!(
        matches!(result, Err(SceneError::InvalidBackdropOuter(4))),
        "outer −1 must be rejected, got {result:?}"
    );

    let lone_outer = {
        let mut b = Scene::builder(64, 64);
        b.root().layer(|m| {
            m.clip(Shape::rect(8.0, 8.0, 56.0, 56.0));
            m.backdrop_outer(2.0);
        });
        b.build()
    };
    let scene_dir = dir.join("lone-outer");
    lone_outer.save(&scene_dir).unwrap();
    let result = Scene::load(&scene_dir);
    assert!(
        matches!(result, Err(SceneError::BackdropOuterWithoutGroup)),
        "a group-less outer must be rejected, got {result:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A backdrop group anchored at the scene root is rejected by validation
/// with the named error — the anchor must be a listed layer so the
/// capture has a paint position.
#[test]
fn a_group_anchor_at_the_scene_root_is_rejected() {
    let mut b = Scene::builder(8, 8);
    b.root().id(NONZERO);
    b.backdrop_group(BackdropGroup {
        anchor: Some(NONZERO),
        ..BackdropGroup::new(1, Vec::new(), 1.0, 1)
    });
    let scene = b.build();
    assert!(
        matches!(
            scene.validate(),
            Err(cherenkov_scene::SceneError::BackdropAnchorAtRoot(1))
        ),
        "unexpected result {:?}",
        scene.validate()
    );
}

/// A backdrop group anchored at a projective layer is rejected: the
/// projective subtree is its own canvas, so the anchor's paint position
/// is not in the members' canvas.
#[test]
fn a_group_anchor_at_a_projective_layer_is_rejected() {
    let mut b = Scene::builder(8, 8);
    b.root().layer(|m| {
        m.projection(cherenkov_scene::Projection {
            matrix: cherenkov_scene::Projection::perspective(100.0),
            ..cherenkov_scene::Projection::default()
        });
        m.id(NONZERO);
    });
    b.backdrop_group(BackdropGroup {
        anchor: Some(NONZERO),
        ..BackdropGroup::new(1, Vec::new(), 1.0, 1)
    });
    let scene = b.build();
    assert!(
        matches!(
            scene.validate(),
            Err(cherenkov_scene::SceneError::BackdropAnchorProjective(1))
        ),
        "unexpected result {:?}",
        scene.validate()
    );
}

/// A backdrop group anchored at a `Layer::id` no layer carries is
/// rejected: the anchor must name a mounted layer.
#[test]
fn a_group_anchor_at_an_unknown_layer_is_rejected() {
    let mut b = Scene::builder(8, 8);
    b.backdrop_group(BackdropGroup {
        anchor: Some(NONZERO),
        ..BackdropGroup::new(1, Vec::new(), 1.0, 1)
    });
    let scene = b.build();
    assert!(
        matches!(
            scene.validate(),
            Err(cherenkov_scene::SceneError::UnknownBackdropAnchor(1))
        ),
        "unexpected result {:?}",
        scene.validate()
    );
}

/// Two layers carrying the same `id` are rejected unconditionally — an
/// anchor could not tell them apart.
#[test]
fn a_duplicate_anchor_layer_id_is_rejected() {
    let mut b = Scene::builder(8, 8);
    b.root().layer(|m| {
        m.id(NONZERO);
    });
    b.root().layer(|m| {
        m.id(NONZERO);
    });
    let scene = b.build();
    assert!(
        matches!(
            scene.validate(),
            Err(cherenkov_scene::SceneError::DuplicateBackdropAnchor(1))
        ),
        "unexpected result {:?}",
        scene.validate()
    );
}

/// A layer `id` of `0` in `scene.json` is a parse error, not a silent
/// `None`: the field is a `NonZeroU32`.
#[test]
fn a_zero_layer_id_is_a_parse_error() {
    let dir = std::env::temp_dir().join(format!(
        "cherenkov-scene-zero-id-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    Scene::builder(8, 8).build().save(&dir).unwrap();
    let path = dir.join("scene.json");
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["root"]["id"] = serde_json::json!(0);
    std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
    let result = Scene::load(&dir);
    assert!(
        matches!(result, Err(cherenkov_scene::SceneError::Json(_))),
        "a zero id must fail parsing, got {result:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
