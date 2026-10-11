//! Uniform buffers for effects that encode into a shared command encoder.
//!
//! A queue write lands before the host submits its encoder, so two encodes
//! of one effect in a frame cannot share a uniform buffer: both recorded
//! passes would read the last write. [`UniformBuffers`] keys its buffers by
//! the exact words they hold and hands each encode a buffer no other encode
//! of the same frame has bound.

extern crate alloc;

use alloc::vec::Vec;

/// The most buffers one uniform block keeps. A recorded bind group keeps
/// its buffer alive, so evicting the oldest entry is safe.
const MAX_BUFFERS: usize = 16;

/// One cached buffer: the words it holds and the frame sequence that last
/// bound it.
#[derive(Debug)]
struct Entry {
    words: Vec<u32>,
    buffer: wgpu::Buffer,
    last_used: u64,
}

/// The uniform buffers of one uniform block, keyed by their contents.
#[derive(Debug)]
pub struct UniformBuffers {
    label: &'static str,
    entries: Vec<Entry>,
}

impl UniformBuffers {
    /// No buffers yet; `label` names every buffer this creates.
    pub const fn new(label: &'static str) -> Self {
        Self {
            label,
            entries: Vec::new(),
        }
    }

    /// The index of the buffer holding `words` for an encode of frame
    /// `sequence`; [`Self::buffer`] returns it.
    ///
    /// A buffer that already holds `words` is reused. On a miss, the
    /// stalest buffer no encode of `sequence` has bound is rewritten — an
    /// earlier sequence's encoder is submitted before the next sequence
    /// encodes — and a new buffer is created only when every buffer is in
    /// use by this sequence.
    pub fn select(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        words: Vec<u32>,
        sequence: u64,
    ) -> usize {
        if let Some(index) = self.entries.iter().position(|entry| entry.words == words) {
            self.entries[index].last_used = sequence;
            index
        } else if let Some((index, _)) = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.last_used < sequence)
            .min_by_key(|(_, entry)| entry.last_used)
        {
            let entry = &mut self.entries[index];
            queue.write_buffer(&entry.buffer, 0, bytemuck::cast_slice(&words));
            entry.words = words;
            entry.last_used = sequence;
            index
        } else {
            if self.entries.len() == MAX_BUFFERS {
                self.entries.remove(0);
            }
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: (words.len() * core::mem::size_of::<u32>()) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: true,
            });
            buffer
                .slice(..)
                .get_mapped_range_mut()
                .expect("buffer range is mapped and not overlapping")
                .copy_from_slice(bytemuck::cast_slice(&words));
            buffer.unmap();
            self.entries.push(Entry {
                words,
                buffer,
                last_used: sequence,
            });
            self.entries.len() - 1
        }
    }

    /// The buffer [`Self::select`] returned `index` for.
    pub fn buffer(&self, index: usize) -> &wgpu::Buffer {
        &self.entries[index].buffer
    }

    /// The number of buffers held — for tests asserting reuse.
    #[cfg(test)]
    pub const fn buffer_count(&self) -> usize {
        self.entries.len()
    }
}
