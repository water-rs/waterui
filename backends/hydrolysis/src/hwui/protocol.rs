//! The HWUI command-buffer wire format.
//!
//! One frame is one little-endian buffer of 32-bit words: a three-word
//! header — [`MAGIC`], the frame sequence and the buffer's byte length —
//! then records. A record is one word holding a `u16` opcode (low half) and
//! a `u16` payload length in words (high half), followed by the payload.
//! Every field is a whole word, so records stay 4-byte aligned and the
//! replayer reads floats in bulk.
//!
//! The Kotlin constants (`Protocol.kt` under `android/hwui/build/`) are
//! generated from this file and `text/wire.rs` by the `hwui::kotlin` test,
//! which Gradle runs with the contract scenes. A fixed-size op's payload length must equal its [`Op::fixed_words`];
//! a variable op's length must equal what its counts imply. The decoder fails
//! on any mismatch and on an unknown opcode. Any change here bumps
//! `JNI_SCHEMA` on both sides.
//!
//! Ids are dense `u32`s from free lists; [`NONE`] names no resource.
//! Colours are Android `ColorLong`s split into a low and a high word.

/// `"HWUI"` read as a little-endian word.
pub const MAGIC: u32 = u32::from_le_bytes(*b"HWUI");
/// Header words: magic, frame sequence, byte length.
pub const HEADER_WORDS: usize = 3;
/// The id that names nothing — no effect, no shader.
pub const NONE: u32 = u32::MAX;

/// Words of an inline shape: a [`shape`] kind and six operands.
pub const SHAPE_WORDS: u16 = 7;
/// Words of an inline paint: a [`paint`] kind, two operand words and an
/// alpha multiplier.
pub const PAINT_WORDS: u16 = 4;
/// Words of an Android `Matrix`, in `Matrix.getValues` order.
pub const MATRIX_WORDS: u16 = 9;
/// The node-property values of an [`Op::SetTransform`] after its node.
pub const TRANSFORM_VALUES: u16 = 10;
/// The most patches one [`Op::Mesh`] carries: as many as fit a record's
/// 65535 payload words after its two counts. Four vertices each, they
/// stay well inside `u16` indices.
pub const MESH_BAND_PATCHES: u32 = 2730;
const _: () = assert!(2 + MESH_BAND_PATCHES * MESH_PATCH_FLOATS <= 0xffff);
/// Words of one [`Op::Mesh`] patch: four corners, four colours.
pub const MESH_PATCH_FLOATS: u32 = 24;

/// The opcodes. The high byte is the group: `0x01` node ops, `0x02`
/// recording ops (only between [`Op::Record`] and [`Op::EndRecord`]),
/// `0x03` resource ops, `0x04` host ops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Op {
    /// `node`. Allocates a `RenderNode` under a fresh id.
    CreateNode = 0x0101,
    /// `node`. Drops the table's reference.
    ReleaseNode = 0x0102,
    /// `node, left, top, right, bottom` (i32), `clip_to_bounds`.
    SetPosition = 0x0103,
    /// `node, translation_x, translation_y, scale_x, scale_y, rotation_z,
    /// pivot_x, pivot_y, rotation_x, rotation_y, camera_distance` (angles in
    /// degrees).
    SetTransform = 0x0104,
    /// `node, alpha`.
    SetAlpha = 0x0105,
    /// `node, kind` ([`clip`]), `left, top, right, bottom` (i32), `radius`.
    SetClip = 0x0106,
    /// `node, blend` ([`blend`]), `force_layer`.
    SetComposite = 0x0107,
    /// `node, effect` (or [`NONE`]).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no lowering emits it until RenderEffect filters land (water-rs/waterui#1750); the contract test exercises its wire form"
        )
    )]
    SetEffect = 0x0108,
    /// `node, width, height` (i32). Opens the node's display list.
    Record = 0x0109,
    /// Closes the open display list.
    EndRecord = 0x010A,

    /// Saves the canvas.
    Save = 0x0201,
    /// Restores the canvas.
    Restore = 0x0202,
    /// A [`MATRIX_WORDS`] matrix.
    Concat = 0x0203,
    /// `left, top, right, bottom`.
    ClipRect = 0x0204,
    /// `path`.
    ClipPath = 0x0205,
    /// Shape, paint.
    Fill = 0x0206,
    /// Shape, paint, stroke style, then `dash_count` intervals.
    Stroke = 0x0207,
    /// Shape, `color_lo, color_hi, blur_radius, dx, dy, spread`: a
    /// positive `spread` strokes the shape `2 × spread` wide with round
    /// joins as well as filling it (`FILL_AND_STROKE`); a primitive arrives
    /// already grown, with `spread` 0.
    Shadow = 0x0208,
    /// `font, size`, paint, `style` ([`glyph_style`]), stroke style,
    /// `dash_count` intervals, `count`, then `count` glyph ids and
    /// `2 × count` positions. A fill has no dashes.
    Glyphs = 0x0209,
    /// `layout`, a [`MATRIX_WORDS`] matrix: draws a platform text layout's
    /// node.
    Text = 0x020A,
    /// `bitmap, left, top, right, bottom, sampling` ([`sampling`]).
    Image = 0x020B,
    /// `interpolation` ([`mesh_interpolation`]), `patch_count` (at most
    /// [`MESH_BAND_PATCHES`]), then per patch its corners 00, 10, 01, 11
    /// as `x, y` and their colours as premultiplied linear extended-sRGB
    /// `r, g, b, a` ([`MESH_PATCH_FLOATS`] words). Draws with
    /// `BlendMode.SRC`, so a later patch replaces an earlier one; it is
    /// recorded only into an isolated node.
    Mesh = 0x020C,
    /// `node`. Draws a child node.
    DrawNode = 0x020D,

    /// `path, fill_type` ([`fill_type`]), `verb_count, point_count`, then
    /// the verbs ([`verb`]) one to a word and `2 × point_count` coordinates.
    DefinePath = 0x0301,
    /// `path`.
    ReleasePath = 0x0302,
    /// `shader, kind` ([`shader`]), a [`MATRIX_WORDS`] local matrix, then the
    /// kind's operands (see [`shader`]).
    DefineShader = 0x0303,
    /// `shader`.
    ReleaseShader = 0x0304,
    /// `font`.
    ReleaseFont = 0x0305,
    /// `bitmap`.
    ReleaseBitmap = 0x0306,
    /// `runtime_shader`.
    ReleaseRuntimeShader = 0x0307,
    /// `effect`.
    ReleaseEffect = 0x0308,
    /// `layout`.
    ReleaseTextLayout = 0x0309,
    /// `font, base, axis_count`, then `axis_count` axis tags and
    /// `axis_count` user-space values: a variation instance of `base`,
    /// released with it.
    DeriveFont = 0x030A,

    /// `count`, then `count` entries of `kind` ([`host`]), `id_lo, id_hi`:
    /// the session's top-level draw order, bottom first.
    HostOrder = 0x0401,
}

impl Op {
    /// Every opcode, in code order.
    #[cfg(test)]
    pub const ALL: [Self; 34] = [
        Self::CreateNode,
        Self::ReleaseNode,
        Self::SetPosition,
        Self::SetTransform,
        Self::SetAlpha,
        Self::SetClip,
        Self::SetComposite,
        Self::SetEffect,
        Self::Record,
        Self::EndRecord,
        Self::Save,
        Self::Restore,
        Self::Concat,
        Self::ClipRect,
        Self::ClipPath,
        Self::Fill,
        Self::Stroke,
        Self::Shadow,
        Self::Glyphs,
        Self::Text,
        Self::Image,
        Self::Mesh,
        Self::DrawNode,
        Self::DefinePath,
        Self::ReleasePath,
        Self::DefineShader,
        Self::ReleaseShader,
        Self::ReleaseFont,
        Self::ReleaseBitmap,
        Self::ReleaseRuntimeShader,
        Self::ReleaseEffect,
        Self::ReleaseTextLayout,
        Self::DeriveFont,
        Self::HostOrder,
    ];

    /// The wire code.
    #[must_use]
    pub const fn code(self) -> u16 {
        self as u16
    }

    /// The payload length of a fixed-size op, in words; `None` for an op
    /// whose length follows from its counts.
    #[must_use]
    pub const fn fixed_words(self) -> Option<u16> {
        Some(match self {
            Self::EndRecord | Self::Save | Self::Restore => 0,
            Self::CreateNode
            | Self::ReleaseNode
            | Self::ClipPath
            | Self::DrawNode
            | Self::ReleasePath
            | Self::ReleaseShader
            | Self::ReleaseFont
            | Self::ReleaseBitmap
            | Self::ReleaseRuntimeShader
            | Self::ReleaseEffect
            | Self::ReleaseTextLayout => 1,
            Self::SetAlpha | Self::SetEffect => 2,
            Self::SetComposite | Self::Record => 3,
            Self::ClipRect => 4,
            Self::SetPosition | Self::Image => 6,
            Self::SetClip => 7,
            Self::Concat => MATRIX_WORDS,
            Self::Text => 1 + MATRIX_WORDS,
            Self::SetTransform => 1 + TRANSFORM_VALUES,
            Self::Fill => SHAPE_WORDS + PAINT_WORDS,
            Self::Shadow => SHAPE_WORDS + 6,
            Self::Stroke
            | Self::Glyphs
            | Self::Mesh
            | Self::DefinePath
            | Self::DefineShader
            | Self::DeriveFont
            | Self::HostOrder => return None,
        })
    }

    /// The op's name in the contract test's event log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CreateNode => "CreateNode",
            Self::ReleaseNode => "ReleaseNode",
            Self::SetPosition => "SetPosition",
            Self::SetTransform => "SetTransform",
            Self::SetAlpha => "SetAlpha",
            Self::SetClip => "SetClip",
            Self::SetComposite => "SetComposite",
            Self::SetEffect => "SetEffect",
            Self::Record => "Record",
            Self::EndRecord => "EndRecord",
            Self::Save => "Save",
            Self::Restore => "Restore",
            Self::Concat => "Concat",
            Self::ClipRect => "ClipRect",
            Self::ClipPath => "ClipPath",
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
            Self::Shadow => "Shadow",
            Self::Glyphs => "Glyphs",
            Self::Text => "Text",
            Self::Image => "Image",
            Self::Mesh => "Mesh",
            Self::DrawNode => "DrawNode",
            Self::DefinePath => "DefinePath",
            Self::ReleasePath => "ReleasePath",
            Self::DefineShader => "DefineShader",
            Self::ReleaseShader => "ReleaseShader",
            Self::ReleaseFont => "ReleaseFont",
            Self::ReleaseBitmap => "ReleaseBitmap",
            Self::ReleaseRuntimeShader => "ReleaseRuntimeShader",
            Self::ReleaseEffect => "ReleaseEffect",
            Self::ReleaseTextLayout => "ReleaseTextLayout",
            Self::DeriveFont => "DeriveFont",
            Self::HostOrder => "HostOrder",
        }
    }
}

codes! {
    /// Inline shape kinds: the first of [`SHAPE_WORDS`].
    shape: u32 => "ShapeKind" {
        /// `left, top, right, bottom`.
        RECT = 0;
        /// `left, top, right, bottom, radius_x, radius_y`.
        ROUND_RECT = 1;
        /// `left, top, right, bottom`.
        OVAL = 2;
        /// `path` (a [`super::Op::DefinePath`] id).
        PATH = 3;
        /// `x0, y0, x1, y1`; stroked only.
        LINE = 4;
    }
}

codes! {
    /// Inline paint kinds: the first of [`PAINT_WORDS`].
    paint: u32 => "PaintKind" {
        /// `color_lo, color_hi`.
        COLOR = 0;
        /// `shader` (a [`super::Op::DefineShader`] id), `0`.
        SHADER = 1;
    }
}

codes! {
    /// [`Op::SetClip`] kinds.
    clip: u32 => "ClipKind" {
        /// No node clip.
        NONE = 0;
        /// `setClipRect`.
        RECT = 1;
        /// A round-rect outline with `setClipToOutline`.
        ROUND_RECT = 2;
    }
}

codes! {
    /// `Path.FillType`.
    fill_type: u32 => "FillType" {
        /// `WINDING`.
        WINDING = 0;
        /// `EVEN_ODD`.
        EVEN_ODD = 1;
    }
}

codes! {
    /// Path verbs and the points each consumes.
    verb: u32 => "Verb" {
        /// One point.
        MOVE = 0;
        /// One point.
        LINE = 1;
        /// Two points.
        QUAD = 2;
        /// Three points.
        CUBIC = 3;
        /// No point.
        CLOSE = 4;
    }
}

codes! {
    /// `Shader.TileMode`.
    tile: u32 => "Tile" {
        /// `CLAMP`.
        CLAMP = 0;
        /// `REPEAT`.
        REPEAT = 1;
        /// `MIRROR`.
        MIRROR = 2;
        /// `DECAL`.
        DECAL = 3;
    }
}

codes! {
    /// A registered bitmap's `Bitmap.Config`.
    bitmap_format: u32 => "BitmapFormat" {
        /// `ARGB_8888`: 8-bit RGBA bytes in memory order.
        ARGB_8888 = 0;
        /// `RGBA_F16`: half-float RGBA.
        RGBA_F16 = 1;
    }
}

codes! {
    /// A registered bitmap's `ColorSpace`.
    color_space: u32 => "BitmapColorSpace" {
        /// sRGB.
        SRGB = 0;
        /// Display P3 with the sRGB transfer function.
        DISPLAY_P3 = 1;
        /// Linear sRGB, extended for `RGBA_F16`.
        LINEAR_SRGB = 2;
        /// Display P3 primaries with a linear transfer function.
        LINEAR_P3 = 3;
    }
}

codes! {
    /// Bitmap sampling.
    sampling: u32 => "SamplingKind" {
        /// Nearest neighbour.
        NEAREST = 0;
        /// Bilinear.
        LINEAR = 1;
    }
}

codes! {
    /// [`Op::DefineShader`] kinds and the operands after the local matrix.
    shader: u32 => "ShaderKind" {
        /// `x0, y0, x1, y1, tile, stop_count`, then `stop_count` colours (two
        /// words each) and `stop_count` positions.
        LINEAR = 0;
        /// `start_x, start_y, start_radius, end_x, end_y, end_radius, tile,
        /// stop_count`, then the stops as for [`LINEAR`].
        RADIAL = 1;
        /// `center_x, center_y, stop_count`, then the stops as for [`LINEAR`].
        SWEEP = 2;
        /// `bitmap, tile_x, tile_y, sampling`.
        BITMAP = 3;
        /// `runtime_shader, uniform_count`, then the uniforms.
        RUNTIME = 4;
    }
}

codes! {
    /// `Paint.Cap`.
    cap: u32 => "CapKind" {
        /// `BUTT`.
        BUTT = 0;
        /// `ROUND`.
        ROUND = 1;
        /// `SQUARE`.
        SQUARE = 2;
    }
}

codes! {
    /// `Paint.Join`.
    join: u32 => "JoinKind" {
        /// `MITER`.
        MITER = 0;
        /// `ROUND`.
        ROUND = 1;
        /// `BEVEL`.
        BEVEL = 2;
    }
}

codes! {
    /// [`Op::Glyphs`] styles.
    glyph_style: u32 => "GlyphStyle" {
        /// Filled glyphs.
        FILL = 0;
        /// Stroked glyphs.
        STROKE = 1;
    }
}

codes! {
    /// [`Op::HostOrder`] entry kinds.
    host: u32 => "HostKind" {
        /// A top-level node run: `id` is a node.
        NODE = 0;
        /// An embedded platform view: `id` is its placement id.
        PLATFORM_VIEW = 1;
    }
}

codes! {
    /// A mesh's colour weights.
    mesh_interpolation: u32 => "MeshInterpolation" {
        /// Bilinear in the patch coordinates.
        LINEAR = 0;
        /// Bilinear in the smoothstepped patch coordinates.
        SMOOTHSTEP = 1;
    }
}

codes! {
    /// `android.graphics.BlendMode`, by explicit code rather than ordinal.
    blend: u32 => "BlendCode" {
        /// `CLEAR`.
        CLEAR = 0;
        /// `SRC`.
        SRC = 1;
        /// `DST`.
        DST = 2;
        /// `SRC_OVER`.
        SRC_OVER = 3;
        /// `DST_OVER`.
        DST_OVER = 4;
        /// `SRC_IN`.
        SRC_IN = 5;
        /// `DST_IN`.
        DST_IN = 6;
        /// `SRC_OUT`.
        SRC_OUT = 7;
        /// `DST_OUT`.
        DST_OUT = 8;
        /// `SRC_ATOP`.
        SRC_ATOP = 9;
        /// `DST_ATOP`.
        DST_ATOP = 10;
        /// `XOR`.
        XOR = 11;
        /// `PLUS`.
        PLUS = 12;
        /// `SCREEN`.
        SCREEN = 14;
        /// `OVERLAY`.
        OVERLAY = 15;
        /// `DARKEN`.
        DARKEN = 16;
        /// `LIGHTEN`.
        LIGHTEN = 17;
        /// `COLOR_DODGE`.
        COLOR_DODGE = 18;
        /// `COLOR_BURN`.
        COLOR_BURN = 19;
        /// `HARD_LIGHT`.
        HARD_LIGHT = 20;
        /// `SOFT_LIGHT`.
        SOFT_LIGHT = 21;
        /// `DIFFERENCE`.
        DIFFERENCE = 22;
        /// `EXCLUSION`.
        EXCLUSION = 23;
        /// `MULTIPLY`.
        MULTIPLY = 24;
        /// `HUE`.
        HUE = 25;
        /// `SATURATION`.
        SATURATION = 26;
        /// `COLOR`.
        COLOR = 27;
        /// `LUMINOSITY`.
        LUMINOSITY = 28;
    }
}
