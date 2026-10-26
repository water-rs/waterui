//! The resources a recording names by plain id — fonts, bitmaps, runtime
//! shaders and effects: their dense wire ids and the releases the next
//! command buffer carries. Platform text layouts keep their ids in the
//! shared `text::TextLayoutIds`, by the same discipline.
//!
//! A registration happens synchronously on the replayer (a JNI call that
//! creates the platform object under the id it is given, see `platform`);
//! a release is queued here and travels in order with the next frame,
//! after every recording that could still name the id. A variation
//! instance of a font is its own platform font, derived in the frame that
//! first draws it and released with its base.
//!
//! A released id returns to the pool only when the frame *after* the one
//! carrying its release drains. The command buffer is single and reused, so
//! encoding a new frame means the replayer consumed the previous one: by
//! then the replayer's slot is free, and a registration under the reused id
//! cannot collide with the registration it replaces.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::HwuiError;
use super::buffer::{CommandBuffer, Field};
use super::fonts::FontInfo;
use super::ids::IdPool;
use super::protocol::Op;

/// One resource kind's id table: which ids are live, and the pool
/// [`Table::recycle`] returns retired ids to.
#[derive(Debug)]
pub(in crate::hwui) struct Table {
    ids: IdPool,
    live: Vec<bool>,
}

impl Table {
    pub(in crate::hwui) const fn new(kind: &'static str) -> Self {
        Self {
            ids: IdPool::new(kind),
            live: Vec::new(),
        }
    }

    pub(in crate::hwui) fn acquire(&mut self) -> Result<u32, HwuiError> {
        let id = self.ids.acquire()?;
        let index = id as usize;
        if self.live.len() <= index {
            self.live.resize(index + 1, false);
        }
        self.live[index] = true;
        Ok(id)
    }

    pub(in crate::hwui) fn is_live(&self, id: u64) -> Option<u32> {
        let id = u32::try_from(id).ok()?;
        self.live
            .get(id as usize)
            .copied()
            .unwrap_or(false)
            .then_some(id)
    }

    /// Marks `id` released, keeping it out of the pool until
    /// [`Table::recycle`]; `false` when it was not live.
    pub(in crate::hwui) fn retire(&mut self, id: u32) -> bool {
        match self.live.get_mut(id as usize) {
            Some(live) if *live => {
                *live = false;
                true
            }
            _ => false,
        }
    }

    /// Returns a retired id to the pool.
    pub(in crate::hwui) fn recycle(&mut self, id: u32) {
        self.ids.release(id);
    }
}

/// The target's resource tables. A recording's plain id *is* its wire id:
/// a registration lives as long as any recording naming it, so a released
/// id is never drawn again before it is reused.
#[derive(Debug)]
pub struct Registry {
    fonts: Table,
    bitmaps: Table,
    runtime_shaders: Table,
    effects: Table,
    /// Each live font's tables, by wire id.
    font_info: Vec<Option<Arc<FontInfo>>>,
    /// A variation instance of a registered font, by base and normalized
    /// coordinates: the platform `Font` derived from the base.
    derived: HashMap<(u32, Arc<[i16]>), u32>,
    /// The derivation's user coordinates, reused.
    settings: Vec<(u32, f32)>,
    tags: Vec<u32>,
    values: Vec<f32>,
    /// Releases the next frame carries.
    pending: Vec<(Kind, u32)>,
    /// Releases the last frame carried, recycled when the next one drains.
    retired: Vec<(Kind, u32)>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// A registrable resource kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A `Font`.
    Font,
    /// A hardware `Bitmap`.
    Bitmap,
    /// An AGSL `RuntimeShader`.
    RuntimeShader,
    /// A `RenderEffect`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no lowering registers one until RenderEffect filters land (water-rs/waterui#1750)"
        )
    )]
    Effect,
}

impl Kind {
    /// The kind's name in errors.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Font => "font",
            Self::Bitmap => "image",
            Self::RuntimeShader => "shader",
            Self::Effect => "filter",
        }
    }

    const fn release_op(self) -> Op {
        match self {
            Self::Font => Op::ReleaseFont,
            Self::Bitmap => Op::ReleaseBitmap,
            Self::RuntimeShader => Op::ReleaseRuntimeShader,
            Self::Effect => Op::ReleaseEffect,
        }
    }
}

impl Registry {
    /// Empty tables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fonts: Table::new("font"),
            bitmaps: Table::new("bitmap"),
            runtime_shaders: Table::new("runtime shader"),
            effects: Table::new("effect"),
            font_info: Vec::new(),
            derived: HashMap::new(),
            settings: Vec::new(),
            tags: Vec::new(),
            values: Vec::new(),
            pending: Vec::new(),
            retired: Vec::new(),
        }
    }

    const fn table(&mut self, kind: Kind) -> &mut Table {
        match kind {
            Kind::Font => &mut self.fonts,
            Kind::Bitmap => &mut self.bitmaps,
            Kind::RuntimeShader => &mut self.runtime_shaders,
            Kind::Effect => &mut self.effects,
        }
    }

    /// A fresh id for a registration of `kind`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::IdsExhausted`] when the kind's ids run out.
    pub fn acquire(&mut self, kind: Kind) -> Result<u32, HwuiError> {
        self.table(kind).acquire()
    }

    /// Recycles `id` at once, with no release: its registration never
    /// reached the replayer.
    pub fn forget(&mut self, kind: Kind, id: u32) {
        let table = self.table(kind);
        if table.retire(id) {
            table.recycle(id);
        }
    }

    /// Records the tables of the font registered under `id`.
    pub fn set_font_info(&mut self, id: u32, info: FontInfo) {
        let slot = id as usize;
        if self.font_info.len() <= slot {
            self.font_info.resize(slot + 1, None);
        }
        self.font_info[slot] = Some(Arc::new(info));
    }

    /// The live font `raw`'s ink bounds, in ems, y up.
    #[must_use]
    pub fn font_bounds(&self, raw: u64) -> Option<[f32; 4]> {
        let id = self.fonts.is_live(raw)?;
        self.font_info
            .get(id as usize)?
            .as_ref()
            .map(|info| info.bounds)
    }

    /// Queues the release of `id` into the next frame. Releasing a font
    /// releases every variation instance derived from it.
    pub fn release(&mut self, kind: Kind, id: u32) {
        if !self.table(kind).retire(id) {
            return;
        }
        self.pending.push((kind, id));
        if kind == Kind::Font {
            if let Some(slot) = self.font_info.get_mut(id as usize) {
                *slot = None;
            }
            let mut derived = Vec::new();
            self.derived.retain(|(base, _), font| {
                let keep = *base != id;
                if !keep {
                    derived.push(*font);
                }
                keep
            });
            for font in derived {
                self.release(Kind::Font, font);
            }
        }
    }

    /// The wire id of the live registration `raw` of `kind`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Unregistered`] naming `layer` when no live
    /// registration of `kind` has that id — a resource registered with
    /// another target, or already released.
    pub fn resolve(&self, kind: Kind, raw: u64, layer: u64) -> Result<u32, HwuiError> {
        let table = match kind {
            Kind::Font => &self.fonts,
            Kind::Bitmap => &self.bitmaps,
            Kind::RuntimeShader => &self.runtime_shaders,
            Kind::Effect => &self.effects,
        };
        table.is_live(raw).ok_or_else(|| HwuiError::Unregistered {
            layer,
            kind: kind.name(),
            id: raw,
        })
    }

    /// The wire id of the font `raw` at the normalized coordinates
    /// `coords`: the font itself at its default instance, otherwise its
    /// variation instance, derived once with a `DeriveFont` written into
    /// `buffer` and kept until the base font is released.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Unregistered`] for a font not registered here,
    /// [`HwuiError::Unsupported`] for coordinates the font has no axes
    /// for, and [`HwuiError::Encoding`] from the buffer.
    pub fn glyph_font(
        &mut self,
        raw: u64,
        coords: &Arc<[i16]>,
        layer: u64,
        buffer: &mut CommandBuffer,
    ) -> Result<u32, HwuiError> {
        let base = self.resolve(Kind::Font, raw, layer)?;
        if coords.iter().all(|&coord| coord == 0) {
            return Ok(base);
        }
        if let Some(&font) = self.derived.get(&(base, coords.clone())) {
            return Ok(font);
        }
        let info = self
            .font_info
            .get(base as usize)
            .cloned()
            .flatten()
            .ok_or_else(|| HwuiError::Unsupported {
                layer,
                what: format!(
                    "draws font {raw} at variation coordinates, but its tables were never read"
                ),
            })?;
        info.user_coordinates(coords, &mut self.settings)
            .map_err(|reason| HwuiError::Unsupported {
                layer,
                what: format!("draws font {raw} at variation coordinates: {reason}"),
            })?;
        let font = self.fonts.acquire()?;
        self.tags.clear();
        self.values.clear();
        for &(tag, value) in &self.settings {
            self.tags.push(tag);
            self.values.push(value);
        }
        let count = u32::try_from(self.tags.len()).map_err(|_| HwuiError::Encoding {
            op: "DeriveFont",
            reason: "too many axes".to_owned(),
        })?;
        buffer.op(
            Op::DeriveFont,
            &[
                Field::U("font", font),
                Field::U("base", base),
                Field::U("axes", count),
                Field::Us("tags", &self.tags),
                Field::Fs("values", &self.values),
            ],
        )?;
        let slot = font as usize;
        if self.font_info.len() <= slot {
            self.font_info.resize(slot + 1, None);
        }
        self.font_info[slot] = Some(info);
        self.derived.insert((base, coords.clone()), font);
        Ok(font)
    }

    /// Recycles the ids the previous frame released and writes the queued
    /// releases into `buffer`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Encoding`] from the buffer.
    pub fn drain_releases(&mut self, buffer: &mut CommandBuffer) -> Result<(), HwuiError> {
        for (kind, id) in std::mem::take(&mut self.retired) {
            self.table(kind).recycle(id);
        }
        for &(kind, id) in &self.pending {
            buffer.op(kind.release_op(), &[Field::U("id", id)])?;
        }
        self.retired = std::mem::take(&mut self.pending);
        Ok(())
    }
}

/// The registry the encoder and the resource owner share. A dropped handle
/// does not lock it: it queues its release on [`Dropped`], which the next
/// frame's [`SharedRegistry::drain_releases`] hands to [`Registry::release`],
/// so a handle dropped while a frame is lowering cannot contend for it.
#[derive(Clone, Debug, Default)]
pub struct SharedRegistry {
    registry: Arc<Mutex<Registry>>,
    dropped: Dropped,
}

/// Releases queued by dropped resource handles.
#[derive(Clone, Debug, Default)]
pub struct Dropped(Arc<Mutex<Vec<(Kind, u32)>>>);

impl Dropped {
    /// Queues the release of `id`.
    pub fn push(&self, kind: Kind, id: u32) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((kind, id));
    }
}

impl SharedRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // Every critical section leaves the tables consistent, so a panic
    // elsewhere that poisoned the lock left nothing half-written.
    /// The tables.
    pub fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where handles queue their releases.
    #[must_use]
    pub fn dropped(&self) -> Dropped {
        self.dropped.clone()
    }

    /// Hands every dropped handle's release to [`Registry::release`], then
    /// drains the releases into `buffer` ([`Registry::drain_releases`]).
    ///
    /// # Errors
    ///
    /// [`HwuiError::Encoding`] from the buffer.
    pub fn drain_releases(&self, buffer: &mut CommandBuffer) -> Result<(), HwuiError> {
        let dropped = std::mem::take(
            &mut *self
                .dropped
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        let mut registry = self.lock();
        for (kind, id) in dropped {
            registry.release(kind, id);
        }
        registry.drain_releases(buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Registry};
    use crate::hwui::buffer::CommandBuffer;

    #[test]
    fn a_release_reaches_the_next_buffer_and_the_id_stops_resolving() {
        let mut registry = Registry::new();
        let font = registry.acquire(Kind::Font).unwrap();
        assert_eq!(
            registry.resolve(Kind::Font, u64::from(font), 1).unwrap(),
            font
        );
        registry.release(Kind::Font, font);
        let error = registry
            .resolve(Kind::Font, u64::from(font), 1)
            .unwrap_err();
        assert!(error.to_string().contains("font"), "{error}");
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        registry.drain_releases(&mut buffer).unwrap();
        assert_eq!(buffer.take_log(), ["Frame sequence=1", "ReleaseFont id=0"]);
    }
    #[test]
    fn a_released_id_is_reused_only_after_the_frame_that_carried_its_release() {
        let mut registry = Registry::new();
        let first = registry.acquire(Kind::Bitmap).unwrap();
        registry.release(Kind::Bitmap, first);
        assert_ne!(registry.acquire(Kind::Bitmap).unwrap(), first);
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        registry.drain_releases(&mut buffer).unwrap();
        assert_ne!(registry.acquire(Kind::Bitmap).unwrap(), first);
        buffer.begin_frame();
        registry.drain_releases(&mut buffer).unwrap();
        assert_eq!(registry.acquire(Kind::Bitmap).unwrap(), first);
    }

    #[test]
    fn a_variation_instance_is_derived_once_and_released_with_its_base() {
        use std::sync::Arc;

        use crate::hwui::fonts::FontInfo;

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-fonts/TestVariable-ABC.ttf"
        );
        let data = std::fs::read(path).expect("run backends/hydrolysis/test-fonts/install.py");
        let info = FontInfo::read(&data, 0).unwrap();
        let mut registry = Registry::new();
        let base = registry.acquire(Kind::Font).unwrap();
        registry.set_font_info(base, info);
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        let raw = u64::from(base);
        assert_eq!(
            registry
                .glyph_font(raw, &Arc::from([0]), 1, &mut buffer)
                .unwrap(),
            base
        );
        let coords: Arc<[i16]> = Arc::from([16384]);
        let derived = registry.glyph_font(raw, &coords, 1, &mut buffer).unwrap();
        assert_ne!(derived, base);
        assert_eq!(
            registry
                .glyph_font(raw, &coords.clone(), 2, &mut buffer)
                .unwrap(),
            derived
        );
        let log = buffer.take_log();
        let derive_ops: Vec<&String> = log
            .iter()
            .filter(|line| line.starts_with("DeriveFont"))
            .collect();
        assert_eq!(derive_ops.len(), 1, "{log:?}");
        assert!(
            derive_ops[0].contains(&format!("font={derived} base={base}")),
            "{}",
            derive_ops[0]
        );
        assert!(registry.font_bounds(u64::from(derived)).is_some());
        let error = registry
            .glyph_font(raw, &Arc::from([1, 2, 3, 4, 5, 6, 7, 8, 9]), 3, &mut buffer)
            .unwrap_err();
        assert!(error.to_string().contains("layer 3"), "{error}");
        registry.release(Kind::Font, base);
        buffer.begin_frame();
        registry.drain_releases(&mut buffer).unwrap();
        let log = buffer.take_log();
        assert!(log.contains(&format!("ReleaseFont id={base}")), "{log:?}");
        assert!(
            log.contains(&format!("ReleaseFont id={derived}")),
            "{log:?}"
        );
        assert!(registry.resolve(Kind::Font, u64::from(derived), 1).is_err());
    }
}
