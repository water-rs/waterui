//! Resource registration: [`HwuiResources`] implements the scene's
//! [`SceneBackend`] and [`ShaderBackend`] for the HWUI target.
//!
//! Each registration takes a dense id from the encoder's [`Registry`]
//! and creates the platform object under it synchronously, through a
//! [`ResourcePlatform`] — the Kotlin `RenderNodeSink` over JNI on a device.
//! Its handle's drop queues the release, which the next frame carries after
//! every recording that could still name the id.
//!
//! [`Registry`]: super::resources::Registry

use std::fmt;

use waterui_graphics::draw::{FontId, ImageId, ImageLimits, ShaderId};
use waterui_graphics::{
    FontSource, Handle, ImageColorSpace, ImageData, PlainId, ResourceError, ResourceHandle, Rgba8,
    Rgba16F, SceneBackend, ShaderBackend, ShaderLanguage, ShaderSource,
};

use super::fonts::FontInfo;
use super::protocol::{bitmap_format, color_space};
use super::resources::{Dropped, Kind, SharedRegistry};

/// A bitmap to upload: texels in memory order, `format` a
/// [`bitmap_format`] code and `color_space` a [`color_space`] code.
#[derive(Debug)]
pub struct BitmapUpload<'a> {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub color_space: u32,
    pub premultiplied: bool,
    pub pixels: &'a [u8],
}

/// A float uniform an AGSL fragment declares, in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgslUniform {
    pub name: String,
    pub floats: u32,
}

/// What creates the platform objects. Every method creates the object under
/// `id` or fails with a description; nothing is created on failure.
pub trait ResourcePlatform: 'static {
    /// The largest bitmap the device's renderer samples.
    fn image_limits(&self) -> ImageLimits;

    /// A `Font` built from a copy of `data`, face `index`.
    fn register_font(&self, id: u32, data: &[u8], index: u32) -> Result<(), String>;

    /// The system font the platform's matching resolves `family` to; `read`
    /// sees its file data and face index. An unknown family is an error.
    fn register_system_font(
        &self,
        id: u32,
        family: &str,
        read: &mut FontReader<'_>,
    ) -> Result<(), String>;

    /// A hardware `Bitmap`, uploaded once.
    fn register_bitmap(&self, id: u32, bitmap: &BitmapUpload<'_>) -> Result<(), String>;

    /// An AGSL program and the float uniforms it declares.
    fn register_runtime_shader(
        &self,
        id: u32,
        source: &str,
        uniforms: &[AgslUniform],
    ) -> Result<(), String>;
}

/// The HWUI target's resource owner.
pub struct HwuiResources {
    platform: Box<dyn ResourcePlatform>,
    registry: SharedRegistry,
}

impl fmt::Debug for HwuiResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HwuiResources").finish_non_exhaustive()
    }
}

/// A registration's handle; its drop queues the release.
struct Registered<I: PlainId> {
    id: I,
    kind: Kind,
    wire: u32,
    dropped: Dropped,
}

impl<I: PlainId> ResourceHandle for Registered<I> {
    type Id = I;

    fn id(&self) -> I {
        self.id
    }
}

impl<I: PlainId> Drop for Registered<I> {
    fn drop(&mut self) {
        self.dropped.push(self.kind, self.wire);
    }
}

/// What reads a system font's file data and face index.
pub type FontReader<'a> = dyn FnMut(&[u8], u32) -> Result<(), String> + 'a;

const fn error(kind: Kind, reason: String) -> ResourceError {
    match kind {
        Kind::Font => ResourceError::Font(reason),
        Kind::Bitmap => ResourceError::Image(reason),
        Kind::RuntimeShader | Kind::Effect => ResourceError::Shader(reason),
    }
}

impl HwuiResources {
    /// A resource owner creating its objects through `platform`, in the
    /// tables of `registry` ([`Encoder::registry`](super::Encoder::registry)).
    pub(crate) fn new(platform: Box<dyn ResourcePlatform>, registry: SharedRegistry) -> Self {
        Self { platform, registry }
    }

    fn register<I: PlainId>(
        &self,
        kind: Kind,
        id: fn(u64) -> I,
        create: impl FnOnce(u32) -> Result<Option<FontInfo>, String>,
    ) -> Result<Handle<I>, ResourceError> {
        let wire = self
            .registry
            .lock()
            .acquire(kind)
            .map_err(|failure| error(kind, failure.to_string()))?;
        // The platform call runs unlocked: a frame lowering meanwhile
        // never sees `wire`, which no recording names yet.
        match create(wire) {
            Ok(info) => {
                if let Some(info) = info {
                    self.registry.lock().set_font_info(wire, info);
                }
                Ok(Handle::new(Registered {
                    id: id(u64::from(wire)),
                    kind,
                    wire,
                    dropped: self.registry.dropped(),
                }))
            }
            Err(reason) => {
                self.registry.lock().forget(kind, wire);
                Err(error(kind, reason))
            }
        }
    }

    fn register_image(
        &self,
        width: u32,
        height: u32,
        format: u32,
        color: ImageColorSpace,
        premultiplied: bool,
        pixels: &[u8],
    ) -> Result<Handle<ImageId>, ResourceError> {
        self.platform.image_limits().check(width, height)?;
        let upload = BitmapUpload {
            width,
            height,
            format,
            color_space: match color {
                ImageColorSpace::Srgb => color_space::SRGB,
                ImageColorSpace::DisplayP3 => color_space::DISPLAY_P3,
                ImageColorSpace::LinearSrgb => color_space::LINEAR_SRGB,
                ImageColorSpace::LinearP3 => color_space::LINEAR_P3,
            },
            premultiplied,
            pixels,
        };
        self.register(Kind::Bitmap, ImageId::new, |id| {
            self.platform.register_bitmap(id, &upload).map(|()| None)
        })
    }
}

impl SceneBackend for HwuiResources {
    fn register_font(&self, source: FontSource) -> Result<Handle<FontId>, ResourceError> {
        match source {
            FontSource::Bytes { data, index } => {
                let info = FontInfo::read(&data, index).map_err(ResourceError::Font)?;
                self.register(Kind::Font, FontId::new, |id| {
                    self.platform.register_font(id, &data, index)?;
                    Ok(Some(info))
                })
            }
            FontSource::System { family } => self.register(Kind::Font, FontId::new, |id| {
                let mut info = None;
                self.platform
                    .register_system_font(id, &family, &mut |data, index| {
                        info = Some(
                            FontInfo::read(data, index)
                                .map_err(|reason| format!("system font `{family}`: {reason}"))?,
                        );
                        Ok(())
                    })?;
                info.map(Some)
                    .ok_or_else(|| format!("system font `{family}` was registered unread"))
            }),
        }
    }

    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Handle<ImageId>, ResourceError> {
        self.register_image(
            data.width(),
            data.height(),
            bitmap_format::ARGB_8888,
            data.color_space,
            data.premultiplied,
            data.data(),
        )
    }

    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Handle<ImageId>, ResourceError> {
        self.register_image(
            data.width(),
            data.height(),
            bitmap_format::RGBA_F16,
            data.color_space,
            data.premultiplied,
            data.data(),
        )
    }

    fn image_limits(&self) -> ImageLimits {
        self.platform.image_limits()
    }
}

impl ShaderBackend for HwuiResources {
    fn register_shader(&self, source: ShaderSource) -> Result<Handle<ShaderId>, ResourceError> {
        let name = shader_name(&source.source);
        if source.language != ShaderLanguage::Agsl {
            return Err(ResourceError::Shader(format!(
                "{:?} shader {name} is not AGSL; the HWUI target draws AGSL shader paint only",
                source.language
            )));
        }
        if source.animated {
            return Err(ResourceError::Shader(format!(
                "AGSL shader {name} samples `uniforms.time`, which the HWUI target does not drive"
            )));
        }
        let uniforms = agsl_uniforms(&source.source)
            .map_err(|reason| ResourceError::Shader(format!("AGSL shader {name}: {reason}")))?;
        self.register(Kind::RuntimeShader, ShaderId::new, |id| {
            self.platform
                .register_runtime_shader(id, &source.source, &uniforms)
                .map(|()| None)
                .map_err(|reason| format!("AGSL shader {name}: {reason}"))
        })
    }
}

/// A shader's name in errors: its first line of code, quoted.
fn shader_name(source: &str) -> String {
    const LONGEST: usize = 64;
    let line = source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("//"))
        .unwrap_or("");
    let cut = line
        .char_indices()
        .nth(LONGEST)
        .map_or(line, |(at, _)| &line[..at]);
    if cut.len() < line.len() {
        format!("`{cut}…`")
    } else {
        format!("`{cut}`")
    }
}

/// The float uniforms `source` declares at global scope, in order.
///
/// # Errors
///
/// A description of a declaration shader paint cannot feed: a child
/// shader, colour filter or blender, a non-float uniform, or a
/// declaration that does not parse.
pub fn agsl_uniforms(source: &str) -> Result<Vec<AgslUniform>, String> {
    let tokens = tokens(source)?;
    let mut uniforms = Vec::new();
    let mut depth = 0usize;
    let mut statement = Vec::new();
    for token in tokens {
        match token {
            "{" => {
                depth += 1;
                statement.clear();
            }
            "}" => {
                depth = depth.checked_sub(1).ok_or("an unbalanced `}`")?;
                statement.clear();
            }
            ";" if depth == 0 => {
                uniform(&statement, &mut uniforms)?;
                statement.clear();
            }
            _ if depth == 0 => statement.push(token),
            _ => {}
        }
    }
    Ok(uniforms)
}

fn uniform(statement: &[&str], out: &mut Vec<AgslUniform>) -> Result<(), String> {
    let Some(at) = statement.iter().position(|&token| token == "uniform") else {
        return Ok(());
    };
    let mut rest = statement[at + 1..].iter().copied();
    let ty = rest.next().ok_or("a `uniform` with no type")?;
    let floats = match ty {
        "float" | "half" => 1,
        "float2" | "half2" | "vec2" => 2,
        "float3" | "half3" | "vec3" => 3,
        "float4" | "half4" | "vec4" | "float2x2" | "half2x2" | "mat2" => 4,
        "float3x3" | "half3x3" | "mat3" => 9,
        "float4x4" | "half4x4" | "mat4" => 16,
        "shader" | "colorFilter" | "blender" => {
            return Err(format!(
                "uniform {ty} samples a child effect; shader paint carries float uniforms only"
            ));
        }
        other => {
            return Err(format!(
                "uniform of type `{other}`; shader paint carries float uniforms only"
            ));
        }
    };
    let rest: Vec<&str> = rest.collect();
    for declarator in rest.split(|&token| token == ",") {
        let (name, count) = match declarator {
            [name] => (*name, 1),
            [name, "[", count, "]"] => (
                *name,
                count
                    .parse::<u32>()
                    .map_err(|_| format!("uniform {name}'s array length `{count}`"))?,
            ),
            _ => {
                return Err(format!(
                    "the uniform declaration `{}`",
                    declarator.join(" ")
                ));
            }
        };
        out.push(AgslUniform {
            name: name.to_owned(),
            floats: floats * count,
        });
    }
    Ok(())
}

/// `source`'s tokens with comments removed: identifiers and numbers, and
/// every other character on its own.
fn tokens(source: &str) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(first) = rest.chars().next() {
        if first.is_whitespace() {
            rest = &rest[first.len_utf8()..];
        } else if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |end| &after[end..]);
        } else if let Some(after) = rest.strip_prefix("/*") {
            let end = after.find("*/").ok_or("an unterminated comment")?;
            rest = &after[end + 2..];
        } else if first.is_ascii_alphanumeric() || first == '_' {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                .unwrap_or(rest.len());
            out.push(&rest[..end]);
            rest = &rest[end..];
        } else {
            let end = first.len_utf8();
            out.push(&rest[..end]);
            rest = &rest[end..];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use waterui_graphics::draw::{ImageLimits, Instant};
    use waterui_graphics::{
        FontSource, ImageColorSpace, ImageData, ResourceError, Rgba8, Rgba16F, SceneBackend,
        ShaderBackend, ShaderSource,
    };

    use super::{
        AgslUniform, BitmapUpload, FontReader, HwuiResources, ResourcePlatform, agsl_uniforms,
    };
    use crate::hwui::Encoder;

    #[derive(Clone, Default)]
    struct Fake {
        calls: Rc<RefCell<Vec<String>>>,
        refuse: bool,
    }

    fn test_font(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-fonts/").to_owned() + name;
        std::fs::read(&path).unwrap_or_else(|error| {
            panic!("{path}: {error}; run `uv run backends/hydrolysis/test-fonts/install.py`")
        })
    }

    impl ResourcePlatform for Fake {
        fn image_limits(&self) -> ImageLimits {
            ImageLimits {
                max_dimension: 64,
                max_texels: 64 * 64,
            }
        }

        fn register_font(&self, id: u32, data: &[u8], index: u32) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("font {id} {} bytes face {index}", data.len()));
            if self.refuse {
                Err("refused".to_owned())
            } else {
                Ok(())
            }
        }

        fn register_system_font(
            &self,
            id: u32,
            family: &str,
            read: &mut FontReader<'_>,
        ) -> Result<(), String> {
            if family != "sans-serif" {
                return Err(format!("no system family `{family}`"));
            }
            self.calls
                .borrow_mut()
                .push(format!("system font {id} {family}"));
            read(&test_font("Roboto-Regular.ttf"), 0)
        }

        fn register_bitmap(&self, id: u32, bitmap: &BitmapUpload<'_>) -> Result<(), String> {
            self.calls.borrow_mut().push(format!(
                "bitmap {id} {}x{} format {} space {} premultiplied {} {} bytes",
                bitmap.width,
                bitmap.height,
                bitmap.format,
                bitmap.color_space,
                bitmap.premultiplied,
                bitmap.pixels.len()
            ));
            Ok(())
        }

        fn register_runtime_shader(
            &self,
            id: u32,
            _source: &str,
            uniforms: &[AgslUniform],
        ) -> Result<(), String> {
            let sizes: Vec<String> = uniforms
                .iter()
                .map(|uniform| format!("{}:{}", uniform.name, uniform.floats))
                .collect();
            self.calls
                .borrow_mut()
                .push(format!("shader {id} {}", sizes.join(",")));
            Ok(())
        }
    }

    fn setup(fake: &Fake) -> (Encoder, HwuiResources) {
        let encoder = Encoder::new(34).unwrap();
        let resources = HwuiResources::new(Box::new(fake.clone()), encoder.registry().clone());
        (encoder, resources)
    }

    fn releases(encoder: &mut Encoder) -> Vec<String> {
        encoder.frame(Instant::now(), 1.0).unwrap();
        encoder
            .take_log()
            .into_iter()
            .filter(|line| line.starts_with("Release"))
            .collect()
    }

    #[test]
    fn bitmaps_upload_once_in_their_config_and_release_on_drop() {
        let fake = Fake::default();
        let (mut encoder, resources) = setup(&fake);
        let rgba8 = resources
            .register_rgba8(
                ImageData::<Rgba8>::new(2, 1, vec![0u8; 8])
                    .unwrap()
                    .color_space(ImageColorSpace::DisplayP3),
            )
            .unwrap();
        let f16 = resources
            .register_rgba16f(
                ImageData::<Rgba16F>::new(1, 1, vec![0u8; 8])
                    .unwrap()
                    .color_space(ImageColorSpace::LinearSrgb)
                    .premultiplied(),
            )
            .unwrap();
        assert_eq!(rgba8.id().raw(), 0);
        assert_eq!(f16.id().raw(), 1);
        assert_eq!(
            *fake.calls.borrow(),
            [
                "bitmap 0 2x1 format 0 space 1 premultiplied false 8 bytes",
                "bitmap 1 1x1 format 1 space 2 premultiplied true 8 bytes",
            ]
        );
        drop(rgba8);
        assert_eq!(releases(&mut encoder), ["ReleaseBitmap id=0"]);
        drop(f16);
        assert_eq!(releases(&mut encoder), ["ReleaseBitmap id=1"]);
        assert_eq!(fake.calls.borrow().len(), 2, "each bitmap uploads once");
    }

    #[test]
    fn an_image_past_the_device_limit_is_refused_before_upload() {
        let fake = Fake::default();
        let (_encoder, resources) = setup(&fake);
        let error = resources
            .register_rgba8(ImageData::<Rgba8>::new(65, 1, vec![0u8; 65 * 4]).unwrap())
            .unwrap_err();
        assert!(
            matches!(error, ResourceError::TooLarge { width: 65, .. }),
            "{error}"
        );
        assert!(fake.calls.borrow().is_empty());
    }

    #[test]
    fn a_shader_that_is_not_agsl_is_refused_naming_it() {
        let fake = Fake::default();
        let (_encoder, resources) = setup(&fake);
        let error = resources
            .register_shader(ShaderSource::wgsl(
                "// tint\nfn shade(uv: vec2<f32>) -> vec4<f32> { return vec4(1.0); }",
            ))
            .unwrap_err();
        let message = error.to_string();
        assert!(matches!(error, ResourceError::Shader(_)), "{message}");
        assert!(message.contains("Wgsl"), "{message}");
        assert!(message.contains("`fn shade(uv: vec2<f32>)"), "{message}");
        assert!(fake.calls.borrow().is_empty());
    }

    #[test]
    fn an_agsl_shader_registers_its_float_uniforms() {
        let fake = Fake::default();
        let (_encoder, resources) = setup(&fake);
        let handle = resources
            .register_shader(ShaderSource::agsl(
                "uniform float2 size; // the box\nlayout(color) uniform half4 tint;\n\
                 /* weights */ uniform float weights[3], gain;\n\
                 half4 main(float2 p) { float uniform_like = 1.0; return tint; }",
            ))
            .unwrap();
        assert_eq!(handle.id().raw(), 0);
        assert_eq!(
            *fake.calls.borrow(),
            ["shader 0 size:2,tint:4,weights:3,gain:1"]
        );
        assert!(
            agsl_uniforms("uniform shader image; half4 main(float2 p) { return image.eval(p); }")
                .unwrap_err()
                .contains("child effect")
        );
        assert!(
            agsl_uniforms("uniform int count;")
                .unwrap_err()
                .contains("`int`")
        );
    }

    #[test]
    fn fonts_register_with_their_bounds_and_free_their_id_on_refusal() {
        let fake = Fake::default();
        let (mut encoder, resources) = setup(&fake);
        let data: Arc<[u8]> = Arc::from(test_font("Roboto-Regular.ttf"));
        let font = resources
            .register_font(FontSource::Bytes {
                data: data.clone(),
                index: 0,
            })
            .unwrap();
        let bounds = encoder
            .registry()
            .lock()
            .font_bounds(font.id().raw())
            .unwrap();
        assert!(bounds[3] > 0.9 && bounds[1] < -0.2, "{bounds:?}");
        let system = resources
            .register_font(FontSource::System {
                family: "sans-serif".to_owned(),
            })
            .unwrap();
        assert_eq!(system.id().raw(), 1);
        let unknown = resources
            .register_font(FontSource::System {
                family: "No Such Family".to_owned(),
            })
            .unwrap_err();
        assert!(unknown.to_string().contains("No Such Family"), "{unknown}");
        let garbage = resources
            .register_font(FontSource::Bytes {
                data: Arc::from(&b"not a font"[..]),
                index: 0,
            })
            .unwrap_err();
        assert!(matches!(garbage, ResourceError::Font(_)), "{garbage}");
        let refusing = Fake {
            refuse: true,
            ..fake
        };
        let refused = HwuiResources::new(Box::new(refusing), encoder.registry().clone())
            .register_font(FontSource::Bytes { data, index: 0 })
            .unwrap_err();
        assert!(refused.to_string().contains("refused"), "{refused}");
        // Neither failure left an id behind, nor queued a release.
        assert_eq!(releases(&mut encoder), [] as [String; 0]);
        drop(font);
        assert_eq!(releases(&mut encoder), ["ReleaseFont id=0"]);
    }
}
