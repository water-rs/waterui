//! The lowering, tested by value ([`plan::outline`]) and rendered through
//! Core Animation's own offscreen path (`CALayer.renderInContext` into a
//! bitmap), so no window is shown. Set `WATERUI_CANVAS_REVIEW_DIR` to keep
//! each rendered case as a PNG there.

use kurbo::{Affine, Cap, Circle, Rect, RoundedRect, Stroke};
use waterui_graphics::backend::draw::{
    BlendMode, ColorStop, Content, ContentChange, Draw as _, Group, LayoutSize, LinearGradient,
    Recorder, Shadow, WorkingColor,
};

use super::plan::{self, Composite, LayerKind, Outline};

/// A linear-light colour from sRGB channels in `0..=1`.
fn rgb(red: f32, green: f32, blue: f32) -> WorkingColor {
    rgba(red, green, blue, 1.0)
}

fn rgba(red: f32, green: f32, blue: f32, alpha: f32) -> WorkingColor {
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    WorkingColor::new([linear(red), linear(green), linear(blue), alpha])
}

/// A small count as a gradient offset.
fn unit(count: usize) -> f32 {
    f32::from(u16::try_from(count).expect("test counts are small"))
}

fn stops(colors: &[WorkingColor]) -> Vec<ColorStop> {
    let last = unit(colors.len() - 1);
    colors
        .iter()
        .enumerate()
        .map(|(i, &color)| ColorStop {
            offset: unit(i) / last,
            color,
        })
        .collect()
}

/// The first change of a recording, which carries the whole list.
fn first_list(content: &mut Content) -> waterui_graphics::backend::draw::DisplayList {
    match content.take_change() {
        Some(ContentChange::Replace(picture)) => picture.display_list().clone(),
        other => panic!("a fresh content sends its whole list first, got {other:?}"),
    }
}

fn record(body: impl FnOnce(&mut Recorder)) -> Content {
    Content::record(&LayoutSize::new(), body)
}

fn outline_of(body: impl FnOnce(&mut Recorder)) -> Vec<Outline> {
    let mut content = record(body);
    plan::outline(&first_list(&mut content))
}

fn leaf(kind: LayerKind) -> Outline {
    Outline {
        kind,
        children: Vec::new(),
    }
}

fn scope(kind: LayerKind, children: Vec<Outline>) -> Outline {
    Outline { kind, children }
}

#[test]
fn a_colour_fill_is_a_shape_layer_and_a_gradient_is_a_painted_layer() {
    let outline = outline_of(|r| {
        r.fill(Rect::new(0., 0., 10., 10.), rgb(1., 0., 0.));
        r.fill(
            Circle::new((5., 5.), 4.),
            LinearGradient {
                stops: stops(&[rgb(1., 0., 0.), rgb(0., 0., 1.)]),
                ..LinearGradient::new((0., 0.), (10., 0.))
            },
        );
        r.stroke(Rect::new(0., 0., 4., 4.), Stroke::new(2.), rgb(0., 1., 0.));
    });
    assert_eq!(
        outline,
        vec![
            leaf(LayerKind::Shape),
            leaf(LayerKind::Paint),
            leaf(LayerKind::Shape),
        ]
    );
}

#[test]
fn scopes_nest_as_layers() {
    let outline = outline_of(|r| {
        r.clip(Rect::new(0., 0., 20., 20.), |r| {
            r.transform(Affine::translate((3., 4.)), |r| {
                r.fill(Rect::new(0., 0., 5., 5.), rgb(1., 0., 0.));
            });
        });
        r.clip(RoundedRect::new(0., 0., 20., 20., 4.), |r| {
            r.group(
                Group {
                    opacity: 0.5,
                    ..Group::new()
                },
                |r| r.fill(Rect::new(0., 0., 5., 5.), rgb(1., 0., 0.)),
            );
        });
        r.group(
            Group {
                blend: BlendMode::Multiply,
                ..Group::new()
            },
            |r| r.shadow(Rect::new(0., 0., 5., 5.), Shadow::new(2., rgb(0., 0., 0.))),
        );
    });
    assert_eq!(
        outline,
        vec![
            scope(
                LayerKind::ClipBounds,
                vec![scope(LayerKind::Transform, vec![leaf(LayerKind::Shape)])]
            ),
            scope(
                LayerKind::ClipMask,
                vec![scope(
                    LayerKind::Group {
                        opacity: 0.5,
                        composite: Composite::Normal
                    },
                    vec![leaf(LayerKind::Shape)]
                )]
            ),
            scope(
                LayerKind::Group {
                    opacity: 1.0,
                    composite: Composite::Filter("multiplyBlendMode")
                },
                vec![leaf(LayerKind::Shadow)]
            ),
        ]
    );
}

/// A group that blends against the canvas beneath it is drawn by Core
/// Graphics together with that content, from its level's first command
/// through the scope holding the group; later commands, and other levels,
/// stay layers. `Dst` hides its group.
#[test]
fn backdrop_groups_draw_with_the_content_beneath_them_in_their_level() {
    let outline = outline_of(|r| {
        r.fill(Rect::new(0., 0., 10., 10.), rgb(1., 0., 0.));
        r.clip(Circle::new((5., 5.), 5.), |r| {
            r.group(Group::new().blend(BlendMode::DestIn), |r| {
                r.fill(Rect::new(0., 0., 5., 5.), rgb(0., 1., 0.));
            });
        });
        r.fill(Rect::new(0., 0., 4., 4.), rgb(0., 0., 1.));
        r.group(Group::new().blend(BlendMode::Multiply), |r| {
            r.fill(Rect::new(0., 0., 5., 5.), rgb(1., 0., 0.));
            r.group(Group::new().blend(BlendMode::Hue), |r| {
                r.fill(Rect::new(0., 0., 5., 5.), rgb(0., 1., 0.));
            });
            r.fill(Rect::new(0., 0., 5., 5.), rgb(0., 0., 1.));
        });
        r.group(Group::new().blend(BlendMode::Dst), |r| {
            r.fill(Rect::new(0., 0., 5., 5.), rgb(0., 1., 0.));
        });
    });
    assert_eq!(
        outline,
        vec![
            leaf(LayerKind::Drawn),
            leaf(LayerKind::Shape),
            scope(
                LayerKind::Group {
                    opacity: 1.0,
                    composite: Composite::Filter("multiplyBlendMode")
                },
                vec![leaf(LayerKind::Drawn), leaf(LayerKind::Shape)]
            ),
            scope(
                LayerKind::Group {
                    opacity: 1.0,
                    composite: Composite::Hidden
                },
                vec![leaf(LayerKind::Shape)]
            ),
        ]
    );
}

/// A stroke whose ends take different caps stays one shape layer: it fills
/// the stroke's outline.
#[test]
fn a_stroke_with_different_caps_is_a_shape_layer() {
    let mut stroke = Stroke::new(4.);
    stroke.start_cap = Cap::Butt;
    stroke.end_cap = Cap::Round;
    let outline = outline_of(|r| {
        r.stroke(
            kurbo::Line::new((0., 0.), (20., 0.)),
            stroke,
            rgb(1., 0., 0.),
        );
    });
    assert_eq!(outline, vec![leaf(LayerKind::Shape)]);
}

#[cfg(target_os = "macos")]
mod render {
    use std::rc::Rc;
    use std::sync::Arc;

    use kurbo::Point;
    use waterui_graphics::backend::draw::{Extend, Interpolation, RadialGradient, SweepGradient};

    use super::super::layers::{Ctx, Wake};
    use super::super::resources::Registry;
    use super::super::tree::Tree;
    use super::*;

    /// A small count as a coordinate.
    fn px(count: usize) -> f64 {
        f64::from(u32::try_from(count).expect("test counts are small"))
    }
    use cocoa_ui::bitmap::{
        bitmap_context, context_image, image_from_rgba, render_layer, scale_to_pixels,
    };
    use cocoa_ui::objc2::AllocAnyThread;
    use cocoa_ui::objc2::rc::Retained;
    use cocoa_ui::objc2_core_foundation::CFRetained;
    use cocoa_ui::objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use cocoa_ui::objc2_core_graphics::CGColor;
    use cocoa_ui::objc2_core_text::CTFont;
    use cocoa_ui::objc2_foundation::NSDictionary;
    use cocoa_ui::objc2_quartz_core::{CALayer, CAShapeLayer};
    use kurbo::Shape as _;
    use waterui_graphics::backend::draw::{
        EvenOdd, Glyph, GlyphRun, GlyphStyle, ImageId, ImagePattern, MeshGradient, Sampling,
    };
    use waterui_graphics::backend::{FontSource, ImageData, Rgba8};
    use waterui_graphics::backend::{Handle, SceneBackend as _};

    use super::super::resources::Backend;

    /// Device pixels per point, unless a case needs 1: `renderInContext`
    /// applies shadow offsets and blur radii in device pixels, not points.
    const SCALE: usize = 2;

    /// Renders `layer` as a y-down host does — a flipped superlayer — over
    /// white, and returns RGBA pixels.
    fn pixels(layer: &CALayer, width: usize, height: usize, scale: usize) -> Vec<u8> {
        let host = CALayer::new();
        host.setBounds(CGRect::new(
            CGPoint::ZERO,
            CGSize::new(px(width), px(height)),
        ));
        host.setPosition(CGPoint::ZERO);
        host.setAnchorPoint(CGPoint::ZERO);
        host.setGeometryFlipped(true);
        host.setBackgroundColor(Some(&super::super::cg::color(WorkingColor::WHITE)));
        host.addSublayer(layer);
        // A layer renders its own geometry unflipped: the flipped host the
        // compositor would show sits inside an unflipped parent.
        let outer = CALayer::new();
        outer.setBounds(host.bounds());
        outer.setPosition(CGPoint::ZERO);
        outer.setAnchorPoint(CGPoint::ZERO);
        outer.addSublayer(&host);
        let mut buffer = vec![0_u8; width * scale * height * scale * 4];
        let context = bitmap_context(&mut buffer, width * scale, height * scale)
            .expect("a bitmap context creates");
        scale_to_pixels(&context, px(scale));
        render_layer(&outer, &context);
        let image = context_image(&context).expect("the bitmap captures");
        drop(image);
        layer.removeFromSuperlayer();
        buffer
    }

    fn save(name: &str, buffer: &[u8], width: usize, height: usize, scale: usize) {
        let Ok(dir) = std::env::var("WATERUI_CANVAS_REVIEW_DIR") else {
            return;
        };
        let image = image_from_rgba(buffer, width * scale, height * scale)
            .expect("the pixels make an image");
        let rep = cocoa_ui::objc2_app_kit::NSBitmapImageRep::initWithCGImage(
            cocoa_ui::objc2_app_kit::NSBitmapImageRep::alloc(),
            &image,
        );
        // SAFETY: PNG encoding of a live bitmap rep.
        let data = unsafe {
            rep.representationUsingType_properties(
                cocoa_ui::objc2_app_kit::NSBitmapImageFileType::PNG,
                &NSDictionary::<cocoa_ui::objc2_foundation::NSString>::new(),
            )
        }
        .expect("PNG encoding succeeds");
        std::fs::create_dir_all(&dir).expect("the review directory creates");
        std::fs::write(format!("{dir}/{name}.png"), data.to_vec()).expect("the PNG writes");
    }

    fn lower(content: &mut Content, registry: &Rc<Registry>, scale: usize) -> Tree {
        let list = first_list(content);
        let fixture = Fixture::new(Rc::clone(registry));
        let tree = Tree::new(&list, fixture.at(px(scale)));
        tree.settle();
        tree
    }

    fn show(name: &str, width: usize, height: usize, body: impl FnOnce(&mut Recorder)) {
        show_at(name, width, height, SCALE, body);
    }

    fn show_at(
        name: &str,
        width: usize,
        height: usize,
        scale: usize,
        body: impl FnOnce(&mut Recorder),
    ) {
        show_with(
            name,
            width,
            height,
            scale,
            &Rc::new(Registry::default()),
            body,
        );
    }

    fn show_with(
        name: &str,
        width: usize,
        height: usize,
        scale: usize,
        registry: &Rc<Registry>,
        body: impl FnOnce(&mut Recorder),
    ) {
        let mut content = record(body);
        let tree = lower(&mut content, registry, scale);
        let buffer = pixels(tree.root(), width, height, scale);
        save(name, &buffer, width, height, scale);
    }

    #[test]
    fn shapes_strokes_and_orientation() {
        show("shapes", 160, 120, |r| {
            r.fill(Rect::new(10., 10., 70., 50.), rgb(0.9, 0.2, 0.2));
            r.fill(Circle::new((110., 30.), 22.), rgb(0.2, 0.4, 0.9));
            let mut dashed = Stroke::new(4.);
            dashed.dash_pattern = [8., 4.].into_iter().collect();
            r.stroke(
                RoundedRect::new(10., 65., 150., 110., 12.),
                dashed,
                rgb(0.1, 0.6, 0.3),
            );
        });
    }

    #[test]
    fn gradients_of_every_kind() {
        show("gradients", 240, 120, |r| {
            r.fill(
                Rect::new(5., 5., 75., 115.),
                LinearGradient {
                    stops: stops(&[rgb(1., 0.9, 0.2), rgb(0.9, 0.1, 0.5), rgb(0.1, 0.2, 0.9)]),
                    ..LinearGradient::new((5., 5.), (75., 115.))
                },
            );
            r.fill(
                Circle::new((120., 60.), 40.),
                RadialGradient {
                    start_center: Point::new(110., 50.),
                    start_radius: 0.,
                    end_center: Point::new(120., 60.),
                    end_radius: 40.,
                    stops: stops(&[rgb(1., 1., 1.), rgb(0.1, 0.5, 0.9)]),
                    extend: Extend::Pad,
                    interpolation: Interpolation::default(),
                },
            );
            r.fill(
                Rect::new(165., 20., 235., 100.),
                SweepGradient {
                    stops: stops(&[
                        rgb(1., 0., 0.),
                        rgb(1., 1., 0.),
                        rgb(0., 1., 0.),
                        rgb(0., 1., 1.),
                        rgb(0., 0., 1.),
                        rgb(1., 0., 1.),
                        rgb(1., 0., 0.),
                    ]),
                    ..SweepGradient::new((200., 60.), 0., std::f64::consts::TAU)
                },
            );
        });
    }

    #[test]
    fn clips_transforms_groups_and_shadows() {
        show("scopes", 240, 120, |r| {
            r.clip(Rect::new(10., 10., 70., 70.), |r| {
                r.fill(Circle::new((60., 60.), 40.), rgb(0.9, 0.5, 0.1));
            });
            r.clip(Circle::new((120., 40.), 30.), |r| {
                r.fill(Rect::new(80., 0., 160., 80.), rgb(0.5, 0.2, 0.8));
            });
            r.transform(Affine::translate((190., 40.)) * Affine::rotate(0.5), |r| {
                r.fill(Rect::new(-20., -20., 20., 20.), rgb(0.1, 0.7, 0.7));
            });
            r.group(
                Group {
                    opacity: 0.5,
                    ..Group::new()
                },
                |r| {
                    r.fill(Rect::new(10., 80., 60., 115.), rgb(1., 0., 0.));
                    r.fill(Rect::new(35., 85., 85., 118.), rgb(0., 0., 1.));
                },
            );
        });
    }

    /// An 8x8 image: red top-left, green top-right, blue bottom-left,
    /// yellow bottom-right, so a flipped or mirrored draw is visible.
    fn quadrants(backend: &Backend) -> Handle<ImageId> {
        let mut bytes = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                bytes.extend_from_slice(match (x < 4, y < 4) {
                    (true, true) => &[255, 0, 0, 255],
                    (false, true) => &[0, 200, 0, 255],
                    (true, false) => &[0, 0, 255, 255],
                    (false, false) => &[255, 220, 0, 255],
                });
            }
        }
        backend
            .register_rgba8(ImageData::<Rgba8>::new(8, 8, bytes).expect("a well-formed image"))
            .expect("the image registers")
    }

    #[test]
    fn images_draw_upright_and_tile_as_patterns() {
        let registry = Rc::new(Registry::default());
        let backend = Backend::new(Rc::clone(&registry), u64::MAX);
        let image = quadrants(&backend);
        show_with("images", 240, 120, SCALE, &registry, |r| {
            r.image(image.id(), Rect::new(10., 10., 90., 90.), Sampling::Nearest);
            r.image(
                image.id(),
                Rect::new(100., 10., 140., 50.),
                Sampling::Linear,
            );
            r.fill(
                Rect::new(150., 10., 230., 110.),
                ImagePattern {
                    image: image.id(),
                    transform: Affine::scale(5.),
                    extend_x: Extend::Repeat,
                    extend_y: Extend::Reflect,
                    sampling: Sampling::Nearest,
                },
            );
            r.transform(Affine::translate((120., 80.)) * Affine::rotate(0.4), |r| {
                r.image(
                    image.id(),
                    Rect::new(-15., -15., 15., 15.),
                    Sampling::Nearest,
                );
            });
        });
    }

    fn half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let fraction = f32::from(bits & 0x3ff);
        sign * match exponent {
            0 => fraction * 2f32.powi(-24),
            31 => f32::INFINITY,
            _ => (1.0 + fraction / 1024.0) * 2f32.powi(exponent - 15),
        }
    }

    /// The mesh pass's own output, composited over white and encoded as
    /// sRGB from the linear values, as a review image.
    #[test]
    fn mesh_gradient_pass_paints_bilinear_patches() {
        use cocoa_ui::mesh::{MeshPass, PIXEL_FORMAT, Placement};
        use cocoa_ui::objc2_metal::{
            MTLDevice as _, MTLOrigin, MTLRegion, MTLSize, MTLTexture as _, MTLTextureDescriptor,
            MTLTextureUsage,
        };
        let pass = MeshPass::new();
        // A 2x2-patch grid whose centre vertex is pulled off-grid and whose
        // second patch row is skewed.
        let points = [
            [10., 10.],
            [100., 10.],
            [190., 14.],
            [14., 80.],
            [92., 66.],
            [186., 74.],
            [6., 150.],
            [110., 140.],
            [194., 150.],
        ];
        let colors = [
            [1., 0.1, 0.1, 1.],
            [1., 0.9, 0.1, 1.],
            [0.1, 0.8, 0.2, 1.],
            [0.9, 0.2, 0.7, 1.],
            [1., 1., 1., 1.],
            [0.1, 0.9, 0.9, 1.],
            [0.2, 0.1, 0.8, 1.],
            [0.6, 0.2, 0.9, 1.],
            [0.1, 0.2, 0.4, 0.4],
        ];
        let mesh = pass.mesh(2, 2, &points, &colors, false);
        let (width, height) = (200_usize, 160_usize);
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                PIXEL_FORMAT,
                width * SCALE,
                height * SCALE,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        let texture = pass
            .device()
            .newTextureWithDescriptor(&descriptor)
            .expect("a render target");
        pass.paint_to_texture(
            &mesh,
            Placement {
                rect: [0., 0., px(width), px(height)],
                inverse: Affine::IDENTITY.as_coeffs(),
            },
            &texture,
        );
        let mut texels = vec![0_u16; width * SCALE * height * SCALE * 4];
        // SAFETY: `texels` holds the texture's 8 bytes per texel.
        unsafe {
            texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                core::ptr::NonNull::new(texels.as_mut_ptr().cast()).expect("non-empty"),
                width * SCALE * 8,
                MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width: width * SCALE,
                        height: height * SCALE,
                        depth: 1,
                    },
                },
                0,
            );
        }
        let encode = |linear: f32| {
            let linear = linear.clamp(0., 1.);
            let encoded = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055_f32.mul_add(linear.powf(1. / 2.4), -0.055)
            };
            // The value is in `0.5..=255.5`.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "rounding a value in 0..=255 to a byte"
            )]
            let byte = encoded.mul_add(255.0, 0.5) as u8;
            byte
        };
        let mut buffer = Vec::with_capacity(texels.len());
        for texel in texels.as_chunks::<4>().0 {
            let alpha = half(texel[3]);
            for &channel in &texel[..3] {
                // Premultiplied over white.
                buffer.push(encode(half(channel) + (1.0 - alpha)));
            }
            buffer.push(255);
        }
        save("mesh", &buffer, width, height, SCALE);
    }

    #[test]
    fn glyph_runs_are_outlines() {
        let registry = Rc::new(Registry::default());
        let backend = Backend::new(Rc::clone(&registry), u64::MAX);
        let font = backend
            .register_font(FontSource::System {
                family: "Helvetica".into(),
            })
            .expect("Helvetica registers");
        let descriptor = registry.font(font.id());
        // SAFETY: the descriptor is live; a null matrix is the identity.
        let face = unsafe { CTFont::with_font_descriptor(&descriptor, 48.0, core::ptr::null()) };
        let text: Vec<u16> = "Canvas".encode_utf16().collect();
        let mut ids = vec![0_u16; text.len()];
        // SAFETY: both buffers hold `text.len()` elements.
        let mapped = unsafe {
            face.glyphs_for_characters(
                core::ptr::NonNull::new(text.as_ptr().cast_mut()).expect("non-empty"),
                core::ptr::NonNull::new(ids.as_mut_ptr()).expect("non-empty"),
                text.len().cast_signed(),
            )
        };
        assert!(mapped, "Helvetica has every glyph of the text");
        let glyphs: Vec<Glyph> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| Glyph {
                id: u32::from(id),
                x: 30.0_f32.mul_add(unit(i), 10.0),
                y: 55.0,
                transform: None,
            })
            .collect();
        let run = |style| GlyphRun {
            font: font.id(),
            size: 48.0,
            coords: Vec::new().into(),
            glyphs: glyphs.clone().into(),
            style,
        };
        show_with("glyphs", 200, 100, SCALE, &registry, |r| {
            r.glyphs(
                run(GlyphStyle::Fill),
                LinearGradient {
                    stops: stops(&[rgb(0.9, 0.2, 0.2), rgb(0.2, 0.2, 0.9)]),
                    ..LinearGradient::new((0., 0.), (200., 0.))
                },
            );
            r.transform(Affine::translate((0., 38.)), |r| {
                r.glyphs(run(GlyphStyle::Stroke(Stroke::new(1.5))), rgb(0., 0., 0.));
            });
        });
    }

    #[test]
    fn group_blends_and_even_odd_paths() {
        show("blends", 240, 120, |r| {
            r.fill(Rect::new(0., 0., 240., 60.), rgb(0.95, 0.85, 0.3));
            r.fill(Rect::new(0., 60., 240., 120.), rgb(0.3, 0.5, 0.9));
            for (i, mode) in [
                BlendMode::Multiply,
                BlendMode::Screen,
                BlendMode::Difference,
            ]
            .into_iter()
            .enumerate()
            {
                let x = 75_f64.mul_add(px(i), 10.);
                r.group(
                    Group {
                        blend: mode,
                        ..Group::new()
                    },
                    |r| r.fill(Circle::new((x + 30., 60.), 28.), rgb(0.8, 0.3, 0.6)),
                );
            }
        });
        let mut star = kurbo::BezPath::new();
        for i in 0..5 {
            let angle =
                std::f64::consts::TAU * f64::from(i * 2) / 5.0 - std::f64::consts::FRAC_PI_2;
            let point = Point::new(
                40_f64.mul_add(angle.cos(), 50.),
                40_f64.mul_add(angle.sin(), 50.),
            );
            if i == 0 {
                star.move_to(point);
            } else {
                star.line_to(point);
            }
        }
        star.close_path();
        show("even_odd", 100, 100, |r| {
            r.fill(EvenOdd(star.clone()), rgb(0.2, 0.6, 0.4));
        });
    }

    #[test]
    fn shadows_with_offset_blur_and_spread() {
        show_at("shadows", 240, 120, 1, |r| {
            r.fill(Rect::new(0., 0., 240., 120.), rgb(0.88, 0.9, 0.95));
            let card = RoundedRect::new(15., 20., 75., 60., 8.);
            r.shadow(
                card,
                Shadow::new(5., rgba(0., 0., 0., 0.6)).offset((4., 6.)),
            );
            r.fill(card, rgb(1., 1., 1.));
            let disc = Circle::new((120., 45.), 22.);
            r.shadow(disc, Shadow::new(2., rgba(0.8, 0., 0., 0.8)).spread(4.));
            r.fill(disc, rgb(1., 1., 1.));
            let tile = Rect::new(170., 20., 220., 70.);
            r.shadow(
                tile,
                Shadow::new(8., rgba(0., 0., 0.5, 0.7)).offset((0., 10.)),
            );
            r.fill(tile, rgb(1., 1., 1.));
        });
    }

    /// The layers directly under `tree`'s root.
    fn top_layers(tree: &Tree) -> Vec<Retained<CALayer>> {
        // SAFETY: the root's sublayers are layers this thread owns.
        let layers = unsafe { tree.root().sublayers() };
        layers.map_or_else(Vec::new, |layers| layers.iter().collect())
    }

    /// What a tree is lowered with, outside a host.
    struct Fixture {
        registry: Rc<Registry>,
        mesh: core::cell::OnceCell<Rc<cocoa_ui::mesh::MeshPass>>,
        /// A test shows mesh paints through [`Tree::settle`] instead.
        wake: Wake,
    }

    impl Fixture {
        fn new(registry: Rc<Registry>) -> Self {
            Self {
                registry,
                mesh: core::cell::OnceCell::new(),
                wake: Arc::new(|| {}),
            }
        }

        const fn at(&self, scale: f64) -> Ctx<'_> {
            Ctx {
                registry: &self.registry,
                scale,
                mesh: &self.mesh,
                wake: &self.wake,
            }
        }
    }

    /// A live operand changes its own layer in place: the layer objects
    /// survive, and only the changed one takes the new colour.
    #[test]
    fn a_live_operand_updates_only_its_layer_in_place() {
        let colour = waterui_core::binding(rgb(1., 0., 0.));
        let mut content = record(|r| {
            r.fill(Rect::new(0., 0., 40., 40.), rgb(0., 0., 1.));
            r.fill(Rect::new(40., 0., 80., 40.), colour.clone());
        });
        let fixture = Fixture::new(Rc::new(Registry::default()));
        let ctx = fixture.at(1.0);
        let mut tree = Tree::new(&first_list(&mut content), ctx);
        let before = top_layers(&tree);
        assert_eq!(before.len(), 2);
        let first_fill = shape_fill(&before[0]);
        let old_fill = shape_fill(&before[1]);

        colour.set(rgb(0., 1., 0.));
        let _ = content.sample(waterui_graphics::backend::draw::Instant::now());
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a changed operand sends a slot update");
        };
        assert_eq!(updates.len(), 1, "only the live fill changes");
        tree.update(&updates, ctx);

        let after = top_layers(&tree);
        assert_eq!(after.len(), 2);
        for (old, new) in before.iter().zip(&after) {
            assert!(same(old, new), "no layer is rebuilt");
        }
        assert_eq!(
            shape_fill(&after[0]),
            first_fill,
            "the static fill is untouched"
        );
        assert_ne!(
            shape_fill(&after[1]),
            old_fill,
            "the live fill takes the new colour"
        );
        assert_eq!(
            shape_fill(&after[1]),
            super::super::cg::color(rgb(0., 1., 0.)),
        );
        save("live_update", &pixels(tree.root(), 80, 40, 1), 80, 40, 1);
    }

    fn same(a: &Retained<CALayer>, b: &Retained<CALayer>) -> bool {
        Retained::as_ptr(a) == Retained::as_ptr(b)
    }

    fn shape_fill(layer: &Retained<CALayer>) -> CFRetained<CGColor> {
        let shape = layer
            .clone()
            .downcast::<CAShapeLayer>()
            .expect("a colour fill is a shape layer");
        shape.fillColor().expect("a fill colour is set").into()
    }

    /// Re-recording a region splices only its commands: the layers of the
    /// commands around it are the same objects afterwards.
    #[test]
    fn a_region_edit_replaces_only_its_layers() {
        let mut region = None;
        let mut content = record(|r| {
            r.fill(Rect::new(0., 0., 30., 30.), rgb(0., 0., 1.));
            region = Some(r.region(|r| r.fill(Rect::new(30., 0., 60., 30.), rgb(1., 0., 0.))));
            r.fill(Rect::new(60., 0., 90., 30.), rgb(0., 0.6, 0.));
        });
        let region = region.expect("a region records");
        let fixture = Fixture::new(Rc::new(Registry::default()));
        let ctx = fixture.at(1.0);
        let mut tree = Tree::new(&first_list(&mut content), ctx);
        let before = top_layers(&tree);
        assert_eq!(before.len(), 3);

        region.record(|r| {
            r.fill(Rect::new(30., 0., 60., 30.), rgb(1., 0.6, 0.));
            r.fill(Circle::new((45., 45.), 12.), rgb(0.6, 0., 0.8));
        });
        let Some(ContentChange::Edit(edit)) = content.take_change() else {
            panic!("a region record sends a structural edit");
        };
        tree.edit(&edit, ctx);

        let after = top_layers(&tree);
        assert_eq!(after.len(), 4, "the region now holds two commands");
        assert!(same(&before[0], &after[0]), "the leading layer survives");
        assert!(same(&before[2], &after[3]), "the trailing layer survives");
        assert!(
            !same(&before[1], &after[1]),
            "the region's own layer is replaced",
        );
        save("region_edit", &pixels(tree.root(), 90, 60, 1), 90, 60, 1);
    }

    /// Every blend mode a compositing filter realizes composites as the
    /// Core Graphics blend mode of the same name: a translucent source
    /// over a translucent backdrop, through `renderInContext`, against
    /// Core Graphics drawing the same two rectangles, within 2/255.
    #[test]
    fn compositing_filters_blend_as_core_graphics_blend_modes() {
        use cocoa_ui::objc2_core_graphics::CGContext;
        let backdrop = rgba(0.9, 0.6, 0.1, 0.8);
        let source = rgba(0.2, 0.4, 0.9, 0.7);
        let (width, height) = (40, 20);
        let modes = [
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::PlusLighter,
        ];
        for mode in modes {
            assert!(matches!(plan::composite(mode), Composite::Filter(_)));
            let mut content = record(|r| {
                r.fill(Rect::new(0., 0., 25., 20.), backdrop);
                r.group(Group::new().blend(mode), |r| {
                    r.fill(Rect::new(15., 0., 40., 20.), source);
                });
            });
            let tree = lower(&mut content, &Rc::new(Registry::default()), 1);
            let layered = transparent_pixels(tree.root(), width, height);
            let mut drawn = vec![0_u8; width * height * 4];
            let bitmap = bitmap_context(&mut drawn, width, height).expect("a bitmap context");
            let fill = |color: WorkingColor, rect: Rect| {
                CGContext::set_fill_color_with_color(
                    Some(&bitmap),
                    Some(&super::super::cg::color(color)),
                );
                CGContext::fill_rect(Some(&bitmap), super::super::cg::rect(rect));
            };
            fill(backdrop, Rect::new(0., 0., 25., 20.));
            CGContext::set_blend_mode(
                Some(&bitmap),
                super::super::replay::blend_mode(mode).expect("a drawn mode"),
            );
            fill(source, Rect::new(15., 0., 40., 20.));
            drop(bitmap);
            let worst = layered
                .iter()
                .zip(&drawn)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .expect("pixels");
            assert!(
                worst <= 2,
                "{mode:?} through its compositing filter differs from Core Graphics by {worst}/255"
            );
        }
    }

    /// Renders `layer` into a transparent bitmap with no flip: for a
    /// comparison against Core Graphics drawing into the same bitmap.
    fn transparent_pixels(layer: &CALayer, width: usize, height: usize) -> Vec<u8> {
        let mut buffer = vec![0_u8; width * height * 4];
        let context = bitmap_context(&mut buffer, width, height).expect("a bitmap context");
        render_layer(layer, &context);
        drop(context);
        buffer
    }

    /// One cell per blend mode a Core Graphics drawn run realizes, with
    /// Multiply (a compositing filter) and Normal for reference: an orange
    /// translucent disc, then a group of the mode holding a blue
    /// translucent square across its right half. Each cell is a group of
    /// its own, so it is the level its mode composites onto: `Clear`,
    /// `Src`, `SrcIn`, `SrcOut`, `DestIn` and `DestAtop` clear the disc
    /// wherever the
    /// square is not, as the engine's destructive operators do.
    #[test]
    fn porter_duff_and_non_separable_groups_blend_against_the_canvas() {
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Clear,
            BlendMode::Src,
            BlendMode::Dst,
            BlendMode::DestOver,
            BlendMode::SrcIn,
            BlendMode::DestIn,
            BlendMode::SrcOut,
            BlendMode::DestOut,
            BlendMode::SrcAtop,
            BlendMode::DestAtop,
            BlendMode::Xor,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ];
        let body = |r: &mut Recorder| {
            for (i, mode) in modes.into_iter().enumerate() {
                let (x, y) = (60. * px(i % 6), 60. * px(i / 6));
                r.group(Group::new(), |r| {
                    r.fill(
                        Circle::new((x + 24., y + 30.), 20.),
                        rgba(0.95, 0.55, 0.1, 0.85),
                    );
                    r.group(Group::new().blend(mode), |r| {
                        r.fill(
                            Rect::new(x + 26., y + 10., x + 54., y + 50.),
                            rgba(0.15, 0.35, 0.95, 0.75),
                        );
                    });
                });
            }
        };
        let outline = outline_of(body);
        for (cell, mode) in outline.iter().zip(modes) {
            let expected = match plan::composite(mode) {
                Composite::Backdrop => vec![leaf(LayerKind::Drawn)],
                composite => vec![
                    leaf(LayerKind::Shape),
                    scope(
                        LayerKind::Group {
                            opacity: 1.0,
                            composite,
                        },
                        vec![leaf(LayerKind::Shape)],
                    ),
                ],
            };
            assert_eq!(cell.children, expected, "{mode:?}");
        }
        show("blend_modes", 360, 180, body);
    }

    /// Strokes whose two ends take different caps: butt to round, square
    /// to butt, dashed round to square, and a gradient stroke whose mask
    /// is the outline.
    #[test]
    fn strokes_with_different_caps() {
        let stroke = |width: f64, start: Cap, end: Cap| {
            let mut stroke = Stroke::new(width);
            stroke.start_cap = start;
            stroke.end_cap = end;
            stroke
        };
        show("caps", 200, 120, |r| {
            let guide = |r: &mut Recorder, y: f64| {
                r.stroke(
                    kurbo::Line::new((30., y - 14.), (30., y + 14.)),
                    Stroke::new(0.5),
                    rgb(0.6, 0.6, 0.6),
                );
                r.stroke(
                    kurbo::Line::new((170., y - 14.), (170., y + 14.)),
                    Stroke::new(0.5),
                    rgb(0.6, 0.6, 0.6),
                );
            };
            guide(r, 20.);
            r.stroke(
                kurbo::Line::new((30., 20.), (170., 20.)),
                stroke(14., Cap::Butt, Cap::Round),
                rgb(0.85, 0.2, 0.2),
            );
            guide(r, 50.);
            r.stroke(
                kurbo::Line::new((30., 50.), (170., 50.)),
                stroke(14., Cap::Square, Cap::Butt),
                rgb(0.2, 0.5, 0.85),
            );
            guide(r, 80.);
            let mut dashed = stroke(8., Cap::Round, Cap::Square);
            dashed.dash_pattern = [24., 14.].into_iter().collect();
            r.stroke(
                kurbo::Line::new((30., 80.), (170., 80.)),
                dashed,
                rgb(0.2, 0.6, 0.3),
            );
            guide(r, 106.);
            r.stroke(
                kurbo::Line::new((30., 106.), (170., 106.)),
                stroke(12., Cap::Round, Cap::Butt),
                LinearGradient {
                    stops: stops(&[rgb(0.9, 0.7, 0.1), rgb(0.6, 0.1, 0.7)]),
                    ..LinearGradient::new((30., 0.), (170., 0.))
                },
            );
        });
    }

    /// Pad-extended image patterns clamp to the image's edge texels: the
    /// quadrant image scaled 5x in the middle of each cell, padded on both
    /// axes, padded across and repeated down, and padded down and
    /// reflected across.
    #[test]
    fn pad_extended_image_patterns_clamp_to_the_edge() {
        let registry = Rc::new(Registry::default());
        let backend = Backend::new(Rc::clone(&registry), u64::MAX);
        let image = quadrants(&backend);
        show_with("pad_patterns", 240, 100, SCALE, &registry, |r| {
            for (i, (x, y)) in [
                (Extend::Pad, Extend::Pad),
                (Extend::Pad, Extend::Repeat),
                (Extend::Reflect, Extend::Pad),
            ]
            .into_iter()
            .enumerate()
            {
                let left = 80f64.mul_add(px(i), 5.);
                r.fill(
                    Rect::new(left, 5., left + 70., 95.),
                    ImagePattern {
                        image: image.id(),
                        transform: Affine::translate((left + 15., 30.)) * Affine::scale(5.),
                        extend_x: x,
                        extend_y: y,
                        sampling: Sampling::Nearest,
                    },
                );
            }
        });
    }

    /// The system's variable San Francisco face, from its file.
    fn system_variable_font() -> (Vec<u8>, usize) {
        let bytes = std::fs::read("/System/Library/Fonts/SFNS.ttf")
            .expect("macOS ships the variable San Francisco face");
        let font = skrifa::FontRef::from_index(&bytes, 0).expect("the face parses");
        let weight = skrifa::MetadataProvider::axes(&font)
            .iter()
            .position(|axis| axis.tag() == skrifa::Tag::new(b"wght"))
            .expect("San Francisco varies its weight");
        (bytes, weight)
    }

    /// The outline of `glyph` at `size` and `coords` as the engine's font
    /// stack draws it, in a y-down space.
    fn reference_outline(bytes: &[u8], glyph: u32, size: f32, coords: &[i16]) -> kurbo::BezPath {
        use skrifa::outline::{DrawSettings, OutlinePen};
        struct Pen(kurbo::BezPath);
        impl OutlinePen for Pen {
            fn move_to(&mut self, x: f32, y: f32) {
                self.0.move_to((f64::from(x), -f64::from(y)));
            }
            fn line_to(&mut self, x: f32, y: f32) {
                self.0.line_to((f64::from(x), -f64::from(y)));
            }
            fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
                self.0.quad_to(
                    (f64::from(cx), -f64::from(cy)),
                    (f64::from(x), -f64::from(y)),
                );
            }
            fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
                self.0.curve_to(
                    (f64::from(x1), -f64::from(y1)),
                    (f64::from(x2), -f64::from(y2)),
                    (f64::from(x), -f64::from(y)),
                );
            }
            fn close(&mut self) {
                self.0.close_path();
            }
        }
        let font = skrifa::FontRef::from_index(bytes, 0).expect("the face parses");
        let coords: Vec<skrifa::raw::types::F2Dot14> = coords
            .iter()
            .map(|&bits| skrifa::raw::types::F2Dot14::from_bits(bits))
            .collect();
        let outline = skrifa::MetadataProvider::outline_glyphs(&font)
            .get(skrifa::GlyphId::new(glyph))
            .expect("the glyph has an outline");
        let mut pen = Pen(kurbo::BezPath::new());
        outline
            .draw(
                DrawSettings::unhinted(
                    skrifa::instance::Size::new(size),
                    skrifa::instance::LocationRef::new(&coords),
                ),
                &mut pen,
            )
            .expect("the outline draws");
        pen.0
    }

    /// A run's normalized variation coordinates reach Core Text as the
    /// instance the engine's own font stack draws: the outline of `H` at
    /// three weights matches skrifa's to a hundredth of a unit at 100
    /// units per em, while the weights differ from each other by far more.
    #[test]
    fn variation_coordinates_reach_core_text_as_the_same_instance() {
        use kurbo::Shape as _;
        let (bytes, weight) = system_variable_font();
        let registry = Rc::new(Registry::default());
        let backend = Backend::new(Rc::clone(&registry), u64::MAX);
        let font = backend
            .register_font(FontSource::Bytes {
                data: bytes.clone().into(),
                index: 0,
            })
            .expect("San Francisco registers");
        let face = skrifa::FontRef::from_index(&bytes, 0).expect("the face parses");
        let glyph = skrifa::MetadataProvider::charmap(&face)
            .map('H')
            .expect("the face maps H")
            .to_u32();
        let axes = skrifa::MetadataProvider::axes(&face).len();
        let mut widths = Vec::new();
        for normalized in [-1.0_f64, 0.37, 1.0] {
            let mut coords = vec![0_i16; axes];
            #[expect(
                clippy::cast_possible_truncation,
                reason = "an F2Dot14 in -1..=1 fits sixteen bits"
            )]
            let bits = (normalized * 16384.0).round().clamp(-16384.0, 16383.0) as i16;
            coords[weight] = bits;
            let run = GlyphRun {
                font: font.id(),
                size: 100.0,
                coords: coords.clone().into(),
                glyphs: vec![Glyph {
                    id: glyph,
                    x: 0.0,
                    y: 0.0,
                    transform: None,
                }]
                .into(),
                style: GlyphStyle::Fill,
            };
            let ours = super::super::glyphs::outline(&registry, &run).bounding_box();
            let theirs = reference_outline(&bytes, glyph, 100.0, &coords).bounding_box();
            for (a, b) in [
                (ours.x0, theirs.x0),
                (ours.y0, theirs.y0),
                (ours.x1, theirs.x1),
                (ours.y1, theirs.y1),
            ] {
                assert!(
                    (a - b).abs() < 0.01,
                    "at normalized weight {normalized}: Core Text {ours:?}, skrifa {theirs:?}"
                );
            }
            widths.push(ours.width());
        }
        assert!(
            widths[0] + 5.0 < widths[2],
            "the weights draw different outlines: {widths:?}"
        );
    }

    /// "Canvas" in the system's variable face at the lightest, regular
    /// and heaviest weights, by normalized coordinate.
    #[test]
    fn glyph_runs_draw_at_their_variation() {
        let (bytes, weight) = system_variable_font();
        let registry = Rc::new(Registry::default());
        let backend = Backend::new(Rc::clone(&registry), u64::MAX);
        let font = backend
            .register_font(FontSource::Bytes {
                data: bytes.into(),
                index: 0,
            })
            .expect("San Francisco registers");
        let descriptor = registry.font(font.id());
        // SAFETY: the descriptor is live; a null matrix is the identity.
        let face = unsafe { CTFont::with_font_descriptor(&descriptor, 36.0, core::ptr::null()) };
        let text: Vec<u16> = "Canvas".encode_utf16().collect();
        let mut ids = vec![0_u16; text.len()];
        // SAFETY: both buffers hold `text.len()` elements.
        let mapped = unsafe {
            face.glyphs_for_characters(
                core::ptr::NonNull::new(text.as_ptr().cast_mut()).expect("non-empty"),
                core::ptr::NonNull::new(ids.as_mut_ptr()).expect("non-empty"),
                text.len().cast_signed(),
            )
        };
        assert!(mapped, "San Francisco has every glyph of the text");
        let axes = {
            let file = std::fs::read("/System/Library/Fonts/SFNS.ttf").expect("the face reads");
            skrifa::MetadataProvider::axes(
                &skrifa::FontRef::from_index(&file, 0).expect("the face parses"),
            )
            .len()
        };
        show_with("glyph_variations", 220, 150, SCALE, &registry, |r| {
            for (row, bits) in [-16384_i16, 0, 16383].into_iter().enumerate() {
                let mut coords = vec![0_i16; axes];
                coords[weight] = bits;
                let glyphs: Vec<Glyph> = ids
                    .iter()
                    .enumerate()
                    .map(|(i, &id)| Glyph {
                        id: u32::from(id),
                        x: 30.0_f32.mul_add(unit(i), 15.0),
                        y: 45.0_f32.mul_add(unit(row), 40.0),
                        transform: None,
                    })
                    .collect();
                r.glyphs(
                    GlyphRun {
                        font: font.id(),
                        size: 36.0,
                        coords: coords.into(),
                        glyphs: glyphs.into(),
                        style: GlyphStyle::Fill,
                    },
                    rgb(0.1, 0.1, 0.15),
                );
            }
        });
    }

    /// A five-point star as one path.
    fn star(center: Point, outer: f64) -> kurbo::BezPath {
        let mut star = kurbo::BezPath::new();
        for i in 0..5 {
            let angle =
                std::f64::consts::TAU * f64::from(i * 2) / 5.0 - std::f64::consts::FRAC_PI_2;
            let point = Point::new(
                outer.mul_add(angle.cos(), center.x),
                outer.mul_add(angle.sin(), center.y),
            );
            if i == 0 {
                star.move_to(point);
            } else {
                star.line_to(point);
            }
        }
        star.close_path();
        star
    }

    /// Shadow spread on shapes with no analytic offset: a nonzero star
    /// grown by 6, the same star even-odd (its pentagon a hole) grown by 3,
    /// a line grown into a capsule, an ellipse shrunk by 6, and a
    /// continuous-corner rectangle grown by 5. Each casts a red shadow
    /// with a 1.5 blur under a white outline of its unspread shape.
    #[test]
    fn shadow_spread_offsets_any_shape() {
        use kurbo::{Ellipse, Line};
        use waterui_graphics::backend::draw::ContinuousRect;
        show_at("spread_shadows", 300, 110, 1, |r| {
            r.fill(Rect::new(0., 0., 300., 110.), rgb(0.88, 0.9, 0.95));
            let red = rgba(0.85, 0.05, 0.05, 0.9);
            let outline = |r: &mut Recorder, shape: kurbo::BezPath| {
                r.stroke(shape, Stroke::new(1.), rgb(1., 1., 1.));
            };
            let nonzero = star(Point::new(45., 55.), 34.);
            r.shadow(nonzero.clone(), Shadow::new(1.5, red).spread(6.));
            outline(r, nonzero);
            let holed = star(Point::new(125., 55.), 34.);
            r.shadow(EvenOdd(holed.clone()), Shadow::new(1.5, red).spread(3.));
            outline(r, holed);
            let line = Line::new((170., 25.), (195., 85.));
            r.shadow(line, Shadow::new(1.5, red).spread(7.));
            outline(
                r,
                kurbo::BezPath::from_vec(line.path_elements(0.1).collect()),
            );
            let ellipse = Ellipse::new((235., 55.), (22., 40.), 0.4);
            r.shadow(ellipse, Shadow::new(1.5, red).spread(-6.));
            outline(r, ellipse.to_path(0.1));
            let continuous = ContinuousRect::new(Rect::new(268., 30., 292., 80.), 8.);
            r.shadow(continuous, Shadow::new(1.5, red).spread(5.));
            outline(r, continuous.to_path(0.1));
        });
    }

    /// A 2x2-patch mesh whose centre vertex is pulled off-grid, with one
    /// translucent corner.
    fn review_mesh() -> MeshGradient {
        let points = vec![
            Point::new(0., 0.),
            Point::new(100., 0.),
            Point::new(200., 4.),
            Point::new(4., 70.),
            Point::new(92., 56.),
            Point::new(196., 64.),
            Point::new(0., 140.),
            Point::new(110., 130.),
            Point::new(200., 140.),
        ];
        let colors = vec![
            rgb(1., 0.1, 0.1),
            rgb(1., 0.9, 0.1),
            rgb(0.1, 0.8, 0.2),
            rgb(0.9, 0.2, 0.7),
            rgb(1., 1., 1.),
            rgb(0.1, 0.9, 0.9),
            rgb(0.2, 0.1, 0.8),
            rgb(0.6, 0.2, 0.9),
            rgba(0.1, 0.2, 0.4, 0.4),
        ];
        MeshGradient::new(2, 2, points, colors).expect("a 2 x 2 grid with its 3 x 3 vertices")
    }

    /// The mesh drawn as layers (left) and by Core Graphics in a run
    /// (right, forced by an empty `Xor` group after it), under a star and
    /// a rotated rounded rectangle. The CPU rasterization is expected to
    /// look as the Metal pass does, patch boundary included.
    #[test]
    fn a_run_draws_a_mesh_like_its_layer() {
        let mesh = review_mesh();
        let half = |r: &mut Recorder, run: bool| {
            r.group(Group::new(), |r| {
                r.fill(star(Point::new(70., 75.), 65.), mesh.clone());
                r.transform(Affine::translate((200., 75.)) * Affine::rotate(0.3), |r| {
                    r.fill(
                        RoundedRect::new(-60., -55., 60., 55., 18.),
                        waterui_graphics::backend::draw::TransformedPaint::new(
                            mesh.clone(),
                            Affine::translate((-60., -55.)) * Affine::scale(0.6),
                        ),
                    );
                });
                if run {
                    r.group(Group::new().blend(BlendMode::Xor), |_| {});
                }
            });
        };
        let body = |r: &mut Recorder| {
            half(r, false);
            r.transform(Affine::translate((290., 0.)), |r| half(r, true));
        };
        let outline = outline_of(body);
        assert_eq!(
            outline[1].children[0].children[0].kind,
            LayerKind::Drawn,
            "the second half is one run"
        );
        show("mesh_run_versus_layers", 580, 150, body);
    }

    /// The CPU rasterization of a mesh equals the Metal pass's output, to
    /// what a viewer cannot see: the mean absolute difference over the
    /// premultiplied half-float readback is under 0.002.
    #[test]
    fn the_cpu_mesh_raster_matches_the_metal_pass() {
        use cocoa_ui::mesh::{MeshPass, PIXEL_FORMAT, Placement};
        use cocoa_ui::objc2_metal::{
            MTLDevice as _, MTLOrigin, MTLRegion, MTLSize, MTLTexture as _, MTLTextureDescriptor,
            MTLTextureUsage,
        };
        let mesh = review_mesh();
        let (points, colors) = plan::grid(&mesh);
        let (width, height) = (220_usize, 150_usize);
        let transform = Affine::translate((6., 4.)) * Affine::rotate(0.2);
        let pass = MeshPass::new();
        let uploaded = pass.mesh(2, 2, &points, &colors, false);
        // SAFETY: a 2D texture descriptor is always valid to construct.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                PIXEL_FORMAT,
                width,
                height,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        let texture = pass
            .device()
            .newTextureWithDescriptor(&descriptor)
            .expect("a render target");
        pass.paint_to_texture(
            &uploaded,
            Placement {
                rect: [0., 0., px(width), px(height)],
                inverse: transform.inverse().as_coeffs(),
            },
            &texture,
        );
        let mut texels = vec![0_u16; width * height * 4];
        // SAFETY: `texels` holds the texture's 8 bytes per texel.
        unsafe {
            texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                core::ptr::NonNull::new(texels.as_mut_ptr().cast()).expect("non-empty"),
                width * 8,
                MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width,
                        height,
                        depth: 1,
                    },
                },
                0,
            );
        }
        let patches = cocoa_ui::patch::patches(2, 2, &points, &colors);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the shader's inverse is f32"
        )]
        let cpu = cocoa_ui::patch::raster(
            &patches,
            false,
            width,
            height,
            transform.inverse().as_coeffs().map(|c| c as f32),
        );
        let mut total = 0.0_f64;
        for (texel, expected) in texels.as_chunks::<4>().0.iter().zip(&cpu) {
            for (channel, expected) in texel.iter().zip(expected) {
                total += f64::from((half(*channel) - expected).abs());
            }
        }
        let mean = total / px(width * height * 4);
        assert!(mean < 0.002, "mean absolute difference {mean}");
    }

    /// A 2x2-patch mesh gradient under a star and a rotated rounded
    /// rectangle, through the lowered layer tree and Core Animation's own
    /// `renderInContext`: the mesh layer shows the pass's surface as its
    /// contents, so a capture of the tree draws it.
    #[test]
    fn mesh_gradients_render_through_the_layer_tree() {
        let mesh = review_mesh();
        let mut content = record(|r| {
            r.fill(star(Point::new(70., 75.), 65.), mesh.clone());
            r.transform(Affine::translate((200., 75.)) * Affine::rotate(0.3), |r| {
                r.fill(
                    RoundedRect::new(-60., -55., 60., 55., 18.),
                    waterui_graphics::backend::draw::TransformedPaint::new(
                        mesh.clone(),
                        Affine::translate((-60., -55.)) * Affine::scale(0.6),
                    ),
                );
            });
        });
        assert_eq!(
            plan::outline(&match content.take_change() {
                Some(ContentChange::Replace(picture)) => picture.display_list().clone(),
                other => panic!("a fresh content sends its whole list first, got {other:?}"),
            }),
            vec![
                leaf(LayerKind::Mesh),
                scope(LayerKind::Transform, vec![leaf(LayerKind::Mesh)]),
            ]
        );
        show("mesh_layers", 280, 150, |r| {
            r.fill(star(Point::new(70., 75.), 65.), mesh.clone());
            r.transform(Affine::translate((200., 75.)) * Affine::rotate(0.3), |r| {
                r.fill(
                    RoundedRect::new(-60., -55., 60., 55., 18.),
                    waterui_graphics::backend::draw::TransformedPaint::new(
                        mesh.clone(),
                        Affine::translate((-60., -55.)) * Affine::scale(0.6),
                    ),
                );
            });
        });
    }

    /// The same content as layers (left) and drawn by Core Graphics in a
    /// run (right, forced by an empty `Xor` group after it, which leaves
    /// its backdrop as it is): a shadowed card, a gradient circle under a
    /// rotated clip, an image, glyphs, a dashed stroke and a translucent
    /// group. The halves are expected to look the same.
    #[test]
    fn a_run_draws_what_its_layers_draw() {
        let registry = Rc::new(Registry::default());
        let backend = Backend::new(Rc::clone(&registry), u64::MAX);
        let image = quadrants(&backend);
        let font = backend
            .register_font(FontSource::System {
                family: "Helvetica".into(),
            })
            .expect("Helvetica registers");
        let descriptor = registry.font(font.id());
        // SAFETY: the descriptor is live; a null matrix is the identity.
        let face = unsafe { CTFont::with_font_descriptor(&descriptor, 22.0, core::ptr::null()) };
        let text: Vec<u16> = "Run".encode_utf16().collect();
        let mut ids = vec![0_u16; text.len()];
        // SAFETY: both buffers hold `text.len()` elements.
        let mapped = unsafe {
            face.glyphs_for_characters(
                core::ptr::NonNull::new(text.as_ptr().cast_mut()).expect("non-empty"),
                core::ptr::NonNull::new(ids.as_mut_ptr()).expect("non-empty"),
                text.len().cast_signed(),
            )
        };
        assert!(mapped, "Helvetica has every glyph of the text");
        let half = |r: &mut Recorder, run: bool| {
            r.group(Group::new(), |r| {
                let card = RoundedRect::new(12., 12., 92., 62., 10.);
                r.shadow(
                    card,
                    Shadow::new(4., rgba(0., 0., 0., 0.6)).offset((3., 5.)),
                );
                r.fill(card, rgb(1., 1., 1.));
                r.transform(Affine::rotate_about(0.3, Point::new(140., 40.)), |r| {
                    r.clip(RoundedRect::new(110., 12., 170., 68., 14.), |r| {
                        r.fill(
                            Circle::new((140., 40.), 34.),
                            RadialGradient {
                                start_center: Point::new(130., 30.),
                                start_radius: 0.,
                                end_center: Point::new(140., 40.),
                                end_radius: 34.,
                                stops: stops(&[rgb(1., 1., 0.8), rgb(0.1, 0.4, 0.9)]),
                                extend: Extend::Pad,
                                interpolation: Interpolation::default(),
                            },
                        );
                    });
                });
                r.image(
                    image.id(),
                    Rect::new(14., 80., 54., 120.),
                    Sampling::Nearest,
                );
                let glyphs: Vec<Glyph> = ids
                    .iter()
                    .enumerate()
                    .map(|(i, &id)| Glyph {
                        id: u32::from(id),
                        x: 14.0_f32.mul_add(unit(i), 68.0),
                        y: 108.0,
                        transform: None,
                    })
                    .collect();
                r.glyphs(
                    GlyphRun {
                        font: font.id(),
                        size: 22.0,
                        coords: Vec::new().into(),
                        glyphs: glyphs.into(),
                        style: GlyphStyle::Fill,
                    },
                    rgb(0.15, 0.15, 0.2),
                );
                let mut dashed = Stroke::new(3.);
                dashed.dash_pattern = [6., 4.].into_iter().collect();
                r.stroke(
                    kurbo::Line::new((120., 92.), (175., 120.)),
                    dashed,
                    rgb(0.1, 0.6, 0.3),
                );
                r.group(Group::new().opacity(0.5), |r| {
                    r.fill(Rect::new(120., 80., 150., 110.), rgb(1., 0., 0.));
                    r.fill(Rect::new(135., 95., 165., 125.), rgb(0., 0., 1.));
                });
                if run {
                    r.group(Group::new().blend(BlendMode::Xor), |_| {});
                }
            });
        };
        let body = |r: &mut Recorder| {
            half(r, false);
            r.transform(Affine::translate((190., 0.)), |r| half(r, true));
        };
        let outline = outline_of(body);
        assert_eq!(outline[1].children[0].children[0].kind, LayerKind::Drawn);
        show_with("run_versus_layers", 380, 135, 1, &registry, body);
    }

    /// A live colour inside a run draws the run again in its own layer;
    /// switching a group's blend between a layer and a run lowers the list
    /// again.
    #[test]
    fn live_operands_inside_a_run_redraw_it() {
        let colour = waterui_core::binding(rgb(1., 0., 0.));
        let blend = waterui_core::binding(Group::new().blend(BlendMode::Xor));
        let mut content = record(|r| {
            r.fill(Rect::new(0., 0., 40., 40.), colour.clone());
            r.group(blend.clone(), |r| {
                r.fill(Rect::new(20., 20., 60., 60.), rgb(0., 0., 1.));
            });
        });
        let fixture = Fixture::new(Rc::new(Registry::default()));
        let ctx = fixture.at(1.0);
        let mut tree = Tree::new(&first_list(&mut content), ctx);
        let before = top_layers(&tree);
        assert_eq!(before.len(), 1, "the fill and the group are one run");
        // SAFETY: reading a live layer's contents.
        let old_contents = unsafe { before[0].contents() }.expect("a run shows its drawing");

        colour.set(rgb(0., 0.7, 0.));
        let _ = content.sample(waterui_graphics::backend::draw::Instant::now());
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a changed operand sends a slot update");
        };
        tree.update(&updates, ctx);
        let after = top_layers(&tree);
        assert_eq!(after.len(), 1);
        assert!(same(&before[0], &after[0]), "the run keeps its layer");
        // SAFETY: reading a live layer's contents.
        let new_contents = unsafe { after[0].contents() }.expect("a run shows its drawing");
        assert!(
            !core::ptr::eq(
                Retained::as_ptr(&old_contents),
                Retained::as_ptr(&new_contents)
            ),
            "the run is drawn again"
        );
        save("run_update", &pixels(tree.root(), 60, 60, 1), 60, 60, 1);

        blend.set(Group::new().blend(BlendMode::Multiply));
        let _ = content.sample(waterui_graphics::backend::draw::Instant::now());
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("a changed group sends a slot update");
        };
        tree.update(&updates, ctx);
        assert_eq!(
            top_layers(&tree).len(),
            2,
            "a filter group is a layer of its own beside the fill"
        );
    }

    /// The canvas the text test draws: the three alignments about one
    /// anchor, a width that wraps, and `label` as live text.
    fn text_gallery(
        label: &waterui::reactive::Binding<String>,
    ) -> waterui::graphics::canvas::Canvas {
        use waterui::SignalExt as _;
        use waterui::graphics::canvas::{Canvas, Path, StrokeStyle, TextPlacement};
        use waterui::text::StyledStr;
        use waterui_core::layout::{HorizontalAlignment, Point as P, Rect as R};

        let styled = |text: &str| StyledStr::from(text.to_owned());
        Canvas::new({
            let label = label.clone();
            move |cx| {
                cx.fill(cx.size().map(R::from_size), rgb(0.08, 0.08, 0.1));
                cx.stroke(
                    Path::builder()
                        .move_to(P::new(180.0, 8.0))
                        .line_to(P::new(180.0, 150.0))
                        .build(),
                    StrokeStyle::new(1.0),
                    rgb(0.9, 0.3, 0.3),
                );
                let rows = [
                    (HorizontalAlignment::Leading, "Leading 左对齐"),
                    (HorizontalAlignment::Center, "Center 中央揃え"),
                    (HorizontalAlignment::Trailing, "Trailing 오른쪽"),
                ];
                for (index, (alignment, text)) in rows.into_iter().enumerate() {
                    let y = f32::from(u8::try_from(index).expect("three rows")).mul_add(30.0, 12.0);
                    cx.text(
                        styled(text),
                        TextPlacement::at(P::new(180.0, y)).aligned(alignment),
                    );
                }
                cx.stroke(
                    Path::builder()
                        .move_to(P::new(20.0, 110.0))
                        .line_to(P::new(160.0, 110.0))
                        .build(),
                    StrokeStyle::new(1.0),
                    rgb(0.3, 0.8, 0.5),
                );
                cx.text(
                    styled("Wrapped text 自动换行的中文、日本語の折り返し、한국어 줄바꿈"),
                    TextPlacement::at(P::new(90.0, 114.0))
                        .aligned(HorizontalAlignment::Center)
                        .width(140.0),
                );
                cx.text(
                    label.map(move |text| styled(&text)),
                    TextPlacement::at(P::new(20.0, 250.0)),
                );
            }
        })
    }
    /// Canvas text through Core Text, rendered: Latin, Chinese, Japanese
    /// and Korean, the three alignments about one anchor, a width that
    /// wraps, and a live text update that edits the installed layer tree.
    #[test]
    fn canvas_text_draws_anchored_aligned_wrapped_and_live() {
        use std::any::Any;

        use cocoa_ui::MainThreadMarker;
        use waterui::View as _;
        use waterui::graphics::canvas::CanvasView;
        use waterui::reactive::binding;
        use waterui_graphics::backend::SceneResources;
        use waterui_graphics::backend::{SceneFrame, SceneUpdate};

        use super::super::text::CoreText;

        // SAFETY: the case lays text out and renders layers offscreen. It
        // creates fonts, attributed strings and `CALayer`s, none of which
        // belongs to a view; no view or window is touched.
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let mut env = waterui::Environment::new();
        let _theme = crate::theme::install(&mut env, cocoa_ui::ColorScheme::Dark);
        let label = binding::<String>(String::from("Live"));
        let canvas = text_gallery(&label);
        let boxed: Box<dyn Any> = Box::new(canvas.body(&env));
        let mut view = *boxed
            .downcast::<CanvasView>()
            .expect("a canvas's body is its view");

        let registry = Rc::new(Registry::default());
        let resources = SceneResources::new(Rc::new(Backend::new(Rc::clone(&registry), u64::MAX)));
        resources.set_text(Rc::new(CoreText::new(mtm, Rc::clone(&registry))));
        let (width, height) = (360_u16, 300_u16);
        let mut names = resources.recording();
        let mut content = Content::record(&LayoutSize::new(), |recorder| {
            let _ = view.build_scene(recorder, &mut names, f32::from(width), f32::from(height));
        });
        let _held = names.finish();
        let fixture = Fixture::new(Rc::clone(&registry));
        let ctx = fixture.at(px(SCALE));
        let mut tree = Tree::new(&first_list(&mut content), ctx);
        tree.settle();
        let (w, h) = (usize::from(width), usize::from(height));
        save(
            "canvas_text_before",
            &pixels(tree.root(), w, h, SCALE),
            w,
            h,
            SCALE,
        );

        label.set(String::from("Updated 更新 更新済み 업데이트"));
        let now = waterui_graphics::backend::draw::Instant::now();
        let update = view.update_scene(
            &resources,
            SceneFrame {
                width: f32::from(width),
                height: f32::from(height),
                time: now,
            },
        );
        assert!(
            matches!(update, SceneUpdate::Retained { .. }),
            "a live text edits the installed recording"
        );
        let _ = content.sample(now);
        let Some(ContentChange::Edit(edit)) = content.take_change() else {
            panic!("a changed text is a structural edit of its region");
        };
        tree.edit(&edit, ctx);
        tree.settle();
        save(
            "canvas_text_after",
            &pixels(tree.root(), w, h, SCALE),
            w,
            h,
            SCALE,
        );
    }

    /// A styled text keeps its chunks' colours, background, underline,
    /// strikethrough, weight and italics through Core Text.
    #[test]
    fn canvas_text_keeps_its_styles() {
        use cocoa_ui::MainThreadMarker;
        use waterui::graphics::canvas::TextPlacement;
        use waterui::graphics::color::Srgb;
        use waterui::text::StyledStr;
        use waterui_core::layout::Point as P;
        use waterui_graphics::backend::SceneResources;

        use super::super::text::CoreText;

        // SAFETY: the case lays text out and renders layers offscreen; it
        // touches no view or window.
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let mut env = waterui::Environment::new();
        let _theme = crate::theme::install(&mut env, cocoa_ui::ColorScheme::Dark);
        let text = StyledStr::from("plain ")
            + StyledStr::from("bold").bold()
            + " "
            + StyledStr::from("italic")
                .italic(true)
                .foreground(Srgb::new(0.4, 0.8, 1.0))
            + " "
            + StyledStr::from("underlined").underline(true)
            + " "
            + StyledStr::from("struck").strikethrough(true)
            + " "
            + StyledStr::from("highlighted")
                .background_color(Srgb::new(0.8, 0.2, 0.2))
                .foreground(Srgb::new(1.0, 1.0, 1.0));

        let registry = Rc::new(Registry::default());
        let resources = SceneResources::new(Rc::new(Backend::new(Rc::clone(&registry), u64::MAX)));
        resources.set_text(Rc::new(CoreText::new(mtm, Rc::clone(&registry))));
        let registered = resources
            .text(text, TextPlacement::at(P::new(0.0, 0.0)), env)
            .expect("Core Text lays the text out")
            .expect("a text with lines draws");
        let mut names = resources.recording();
        let layout = names
            .name(registered.layout())
            .expect("registered on this target");
        let mut content = record(|r| {
            r.fill(Rect::new(0., 0., 480., 40.), rgb(0.08, 0.08, 0.1));
            r.text(layout, Affine::translate((10., 8.)));
        });
        let _held = names.finish();
        assert_eq!(
            plan::outline(&first_list(&mut content)),
            vec![leaf(LayerKind::Shape), leaf(LayerKind::Picture)],
            "a text is a picture layer beside the fill"
        );
        let mut content = record(|r| {
            r.fill(Rect::new(0., 0., 480., 40.), rgb(0.08, 0.08, 0.1));
            r.text(layout, Affine::translate((10., 8.)));
        });
        let tree = lower(&mut content, &registry, SCALE);
        save(
            "canvas_text_styles",
            &pixels(tree.root(), 480, 40, SCALE),
            480,
            40,
            SCALE,
        );
    }

    /// The `CanvasView` a canvas-composed view mounts as.
    fn canvas_view(view: impl waterui_core::View) -> waterui_graphics::canvas::CanvasView {
        use waterui_core::{AnyView, Environment, View as _};
        let env = Environment::new();
        let canvas = AnyView::new(view.body(&env))
            .downcast::<waterui_graphics::canvas::Canvas>()
            .unwrap_or_else(|_| panic!("the view composes to a canvas"));
        AnyView::new(canvas.body(&env))
            .downcast::<waterui_graphics::canvas::CanvasView>()
            .map_or_else(|_| panic!("a canvas mounts as a canvas view"), |view| *view)
    }

    /// Linear, radial and mesh gradients authored through the canvas, each
    /// placed by a non-uniform scale, a rotation and a move (nested paint
    /// transforms), on a fill (top row) and a thick stroke (bottom row). The
    /// radial gradient is an ellipse and the shapes stay where they are.
    #[test]
    fn transformed_canvas_paints_move_the_paint_and_not_the_shape() {
        use waterui_core::layout::{Affine2, Point as LPoint, Rect as LRect, Size as LSize};
        use waterui_core::{Environment, View as _};
        use waterui_graphics::backend::SceneResources;
        use waterui_graphics::canvas::{
            Canvas, LinearGradient as CLinear, MeshGradient as CMesh, Paint,
            RadialGradient as CRadial, StrokeStyle,
        };
        use waterui_graphics::color::WorkingColor;

        let color = |r: f32, g: f32, b: f32| WorkingColor::new([r, g, b, 1.0]);
        let rect = |x: f32, y: f32, w: f32, h: f32| LRect::new(LPoint::new(x, y), LSize::new(w, h));
        let canvas = Canvas::new(move |cx| {
            for (column, x) in [60.0_f32, 180.0, 300.0].into_iter().enumerate() {
                for (row, y) in [60.0_f32, 170.0].into_iter().enumerate() {
                    let paint = match column {
                        0 => Paint::from(
                            CLinear::new(LPoint::new(-50.0, 0.0), LPoint::new(50.0, 0.0))
                                .stop(0.0, color(1.0, 0.2, 0.2))
                                .stop(1.0, color(0.2, 0.3, 1.0)),
                        ),
                        1 => Paint::from(
                            CRadial::new(LPoint::zero(), 50.0)
                                .stop(0.0, color(1.0, 1.0, 1.0))
                                .stop(1.0, color(0.1, 0.6, 0.2)),
                        ),
                        _ => Paint::from(CMesh::new(
                            1,
                            1,
                            vec![
                                LPoint::new(-50.0, -50.0),
                                LPoint::new(50.0, -50.0),
                                LPoint::new(-50.0, 50.0),
                                LPoint::new(50.0, 50.0),
                            ],
                            vec![
                                color(1.0, 0.0, 0.0),
                                color(0.0, 1.0, 0.0),
                                color(0.0, 0.0, 1.0),
                                color(1.0, 1.0, 0.0),
                            ],
                        )),
                    }
                    .transformed(Affine2::scale(1.8, 0.6))
                    .transformed(Affine2::rotate(0.5))
                    .transformed(Affine2::translate(x, y));
                    if row == 0 {
                        cx.fill(rect(x - 50.0, y - 50.0, 100.0, 100.0), paint);
                    } else {
                        cx.stroke(
                            rect(x - 42.0, y - 42.0, 84.0, 84.0),
                            StrokeStyle::new(16.0),
                            paint,
                        );
                    }
                }
            }
        });
        let env = Environment::new();
        let mut scene = *waterui_core::AnyView::new(canvas.body(&env))
            .downcast::<waterui_graphics::canvas::CanvasView>()
            .unwrap_or_else(|_| panic!("a canvas mounts as a canvas view"));
        let registry = Rc::new(Registry::default());
        let resources = SceneResources::new(Rc::new(Backend::new(Rc::clone(&registry), u64::MAX)));
        let mut names = resources.recording();
        let mut content = Content::record(&LayoutSize::new(), |recorder| {
            let _ = scene.build_scene(recorder, &mut names, 360.0, 240.0);
        });
        let _held = names.finish();
        let fixture = Fixture::new(registry);
        let tree = Tree::new(&first_list(&mut content), fixture.at(1.0));
        tree.settle();
        save(
            "transformed_canvas_paints",
            &pixels(tree.root(), 360, 240, 1),
            360,
            240,
            1,
        );
    }

    /// Four morphs at five progress steps in a square and a wide box, laid
    /// out as the Hydrolysis gallery of the same name lays them out
    /// (`testing/src/tests/shape_morph.rs`): both draw the path the shape
    /// crate derives, so the sheets are expected to look the same.
    #[test]
    fn morph_sequences_render_in_square_and_wide_boxes() {
        use waterui::shape::{
            Capsule, Circle, Ellipse, MorphShape, Rectangle, RoundedRectangle, ShapeExt as _,
        };
        use waterui_graphics::backend::SceneResources;
        use waterui_graphics::backend::draw::LayoutSize;
        use waterui_graphics::color::Color;

        type Morph = fn(Color) -> MorphShape;
        const GAP: f32 = 8.0;
        const MARGIN: f32 = 16.0;
        const WIDTH: usize = 608;
        const HEIGHT: usize = 584;
        let morphs: [Morph; 4] = [
            |color| Rectangle.morph_to(Circle, color),
            |color| Circle.morph_to(Ellipse, color),
            |color| Capsule.morph_to(Ellipse, color),
            |color| RoundedRectangle::new(0.2).morph_to(Capsule, color),
        ];
        let boxes = [(64.0_f32, 64.0_f32), (112.0, 64.0)];
        let steps = [0.0_f32, 0.25, 0.5, 0.75, 1.0];
        let sheet_width = f32::from(u16::try_from(WIDTH).expect("the sheet is narrow"));

        let registry = Rc::new(Registry::default());
        let resources = SceneResources::new(Rc::new(Backend::new(Rc::clone(&registry), u64::MAX)));
        let fixture = Fixture::new(Rc::clone(&registry));
        let ctx = fixture.at(px(SCALE));
        let sheet = CALayer::new();
        let mut trees = Vec::new();
        let mut y = MARGIN;
        for morph in morphs {
            for (width, height) in boxes {
                let row = 4.0f32.mul_add(GAP, 5.0 * width);
                let mut x = MARGIN + (2.0f32.mul_add(-MARGIN, sheet_width) - row) / 2.0;
                for step in steps {
                    let mut scene = canvas_view(morph(Color::srgb_hex("#3B82F6")).progress(step));
                    let mut names = resources.recording();
                    let mut content = Content::record(&LayoutSize::new(), |recorder| {
                        scene.build_scene(recorder, &mut names, width, height);
                    });
                    let _held = names.finish();
                    let tree = Tree::new(&first_list(&mut content), ctx);
                    tree.settle();
                    tree.root().setAnchorPoint(CGPoint::ZERO);
                    tree.root()
                        .setPosition(CGPoint::new(f64::from(x), f64::from(y)));
                    sheet.addSublayer(tree.root());
                    trees.push(tree);
                    x += width + GAP;
                }
                y += height + GAP;
            }
        }
        // The trees own the layers' display lists, so they live until the
        // sheet is rendered.
        assert_eq!(
            trees.len(),
            morphs.len() * boxes.len() * steps.len(),
            "every cell is on the sheet"
        );
        save(
            "morph_sequences",
            &pixels(&sheet, WIDTH, HEIGHT, SCALE),
            WIDTH,
            HEIGHT,
            SCALE,
        );
    }

    /// The animated mesh gradient, driven as a host drives it: recorded at
    /// mount, then updated in place by frames presented ten seconds apart.
    /// Both captures go through the layer tree and Core Animation's own
    /// `renderInContext`.
    #[test]
    fn the_animated_mesh_gradient_flows_with_the_canvas_clock() {
        use core::time::Duration;
        use waterui_graphics::backend::SceneResources;
        use waterui_graphics::backend::draw::{Instant, LayoutSize};
        use waterui_graphics::backend::{NextFrame, SceneFrame, SceneUpdate};
        use waterui_graphics::gradient::{AnimatedMeshGradient, AnimatedMeshGradientConfig};

        const WIDTH: usize = 360;
        const HEIGHT: usize = 240;
        const WIDTH_PT: f32 = 360.0;
        const HEIGHT_PT: f32 = 240.0;
        let mut scene = canvas_view(AnimatedMeshGradient::new(
            AnimatedMeshGradientConfig::aqua_bloom().speed(1.0),
        ));
        let registry = Rc::new(Registry::default());
        let resources = SceneResources::new(Rc::new(Backend::new(Rc::clone(&registry), u64::MAX)));
        let mut names = resources.recording();
        let mut content = Content::record(&LayoutSize::new(), |recorder| {
            let next = scene.build_scene(recorder, &mut names, WIDTH_PT, HEIGHT_PT);
            assert_eq!(
                next,
                NextFrame::Perpetual,
                "a gradient bound to the clock asks for every frame, with no end"
            );
        });
        let _held = names.finish();
        let fixture = Fixture::new(registry);
        let ctx = fixture.at(1.0);
        let mut tree = Tree::new(&first_list(&mut content), ctx);
        tree.settle();
        save(
            "animated_mesh_t0",
            &pixels(tree.root(), WIDTH, HEIGHT, 1),
            WIDTH,
            HEIGHT,
            1,
        );

        let start = Instant::now();
        for time in [start, start + Duration::from_secs(10)] {
            let update = scene.update_scene(
                &resources,
                SceneFrame {
                    width: WIDTH_PT,
                    height: HEIGHT_PT,
                    time,
                },
            );
            assert_eq!(
                update,
                SceneUpdate::Retained {
                    next: NextFrame::Perpetual
                }
            );
            let _ = content.sample(time);
        }
        let Some(ContentChange::Update(updates)) = content.take_change() else {
            panic!("the clock updates the mesh operand in place");
        };
        tree.update(&updates, ctx);
        tree.settle();
        save(
            "animated_mesh_t10",
            &pixels(tree.root(), WIDTH, HEIGHT, 1),
            WIDTH,
            HEIGHT,
            1,
        );
    }
}
