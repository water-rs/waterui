//! The engine-neutral scene format of the Cherenkov correctness and
//! performance suite.
//!
//! A [`Scene`] describes a single frame: a pixel size, a clear colour, and a
//! layer tree of draw commands. It is deliberately independent of any
//! renderer — Vello (classic, hybrid and CPU), Skia, and eventually Cherenkov
//! itself are driven through adapters in `cherenkov-bench`.
//!
//! A scene is stored on disk as a directory containing `scene.json` plus a
//! `resources/` directory of content-addressed blobs (fonts, images) named by
//! their BLAKE3 hash in lowercase hex. Every scene declares the [`Feature`]s
//! it uses so that an adapter can report a scene as unsupported instead of
//! silently emulating it.

mod backdrop;
mod builder;
mod color;
mod draw;
mod error;
mod layer;
mod scene;
mod shape;
mod text;

#[cfg(feature = "generator")]
#[doc(hidden)]
pub mod corpus;

pub use backdrop::{BackdropEffectSpec, BackdropFilter, BackdropGroup};
pub use builder::{LayerBuilder, SceneBuilder};
pub use color::{Color, ColorSpace};
pub use draw::{
    BlendMode, BlendSpace, Draw, Extend, FillRule, Glyph, GlyphRun, GradientStop, ImageColorSpace,
    ImageEncoding, ImagePaint, LinearGradient, MeshColorInterpolation, MeshGradient,
    MeshGradientError, NormalizedCoord, Paint, RadialGradient, Sampling, StrokeStyle,
    SweepGradient,
};
pub use error::SceneError;
pub use layer::{
    FilterBlend, Group, GroupItem, Item, Layer, LayerFilter, Live, Motion, MotionAnimation,
    Projection,
};
pub use scene::{Feature, Scene, WorkingSpace};
pub use shape::{ContinuousRect, Shape};
#[cfg(feature = "text")]
pub use text::{FontResources, ShapedText};
pub use text::{TextDecoration, TextSource, TextSpan};

/// A BLAKE3 content hash naming a blob in a scene's `resources/` directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceHash(pub blake3::Hash);

impl PartialOrd for ResourceHash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ResourceHash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_bytes().cmp(other.0.as_bytes())
    }
}

impl ResourceHash {
    /// Hash `bytes`, returning the content address.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes))
    }

    /// The lowercase hex file name of this resource inside `resources/`.
    #[must_use]
    pub fn file_name(self) -> String {
        self.0.to_hex().to_string()
    }
}

impl std::fmt::Display for ResourceHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.to_hex())
    }
}

impl std::str::FromStr for ResourceHash {
    type Err = std::array::TryFromSliceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode_nibbleless(s);
        let arr: [u8; 32] = bytes.as_slice().try_into()?;
        Ok(Self(blake3::Hash::from_bytes(arr)))
    }
}

impl serde::Serialize for ResourceHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for ResourceHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse()
            .map_err(|_| serde::de::Error::custom("invalid BLAKE3 hash"))
    }
}

mod hex {
    /// Minimal hex decoder; the resource directory stores hash hex without a
    /// `0x` prefix. Odd lengths and non-hex digits map to an empty vector so
    /// the slice conversion in `FromStr` reports the error.
    pub fn decode_nibbleless(s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(s.len() / 2);
        let mut it = s.bytes();
        while let (Some(h), Some(l)) = (it.next(), it.next()) {
            match (nibble(h), nibble(l)) {
                (Some(h), Some(l)) => out.push(h << 4 | l),
                _ => return Vec::new(),
            }
        }
        if it.next().is_some() {
            return Vec::new();
        }
        out
    }

    const fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
}

pub use kurbo;
