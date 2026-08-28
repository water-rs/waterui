//! The flush boundary: where rasterized regions leave the renderer.
//!
//! Everything above this trait is platform-independent; everything below it
//! is a concrete screen. On desktop the implementation assembles regions
//! into an in-memory framebuffer for snapshot tests; on embedded targets it
//! converts to the panel's pixel format (e.g. RGB565) and streams the
//! region over the display bus, ideally via DMA, without ever holding a
//! full frame in memory.

use crate::compositor::DeviceRegion;

/// Receives rasterized regions from the band compositor.
///
/// Pixels arrive as premultiplied RGBA8 in row-major order with a stride of
/// `region.width * 4` bytes — exactly the scratch pixmap produced by the
/// painter, with no padding between rows.
pub trait DisplayFlush {
    /// Screen size in device pixels as `(width, height)`.
    fn size(&self) -> (u32, u32);

    /// Writes one rasterized region to the screen.
    ///
    /// Called once per scheduled band region, in top-to-bottom order.
    /// `pixels.len()` is exactly `region.area() * 4`.
    fn flush_region(&mut self, region: DeviceRegion, pixels: &[u8]);

    /// Commits all regions flushed since the previous call as one visible
    /// frame.
    ///
    /// Panels that latch on write need no work here; the default is a
    /// no-op.
    fn present(&mut self) {}
}

/// Device-side sink for one RGB565 band.
///
/// Implementations own the actual panel transaction: set the target window,
/// transfer the provided RGB565 pixels, and return only when the slice can be
/// reused. Dew handles RGBA conversion and bounds the shared DMA buffer to the
/// largest scheduled band.
pub trait Rgb565Sink {
    /// Panel size in device pixels as `(width, height)`.
    fn size(&self) -> (u32, u32);

    /// Transfers one RGB565 region in row-major order.
    fn write_region(&mut self, region: DeviceRegion, pixels: &[u16]);

    /// Commits the regions written since the previous call.
    fn present(&mut self) {}
}

/// Converts Dew's RGBA bands into a reusable RGB565 DMA band and streams them
/// to `S`.
#[derive(Debug)]
pub struct Rgb565Display<S> {
    sink: S,
    dma: Vec<u16>,
    peak_rgba_bytes: usize,
    peak_dma_bytes: usize,
}

impl<S> Rgb565Display<S> {
    /// Wraps an RGB565 device sink.
    #[must_use]
    pub const fn new(sink: S) -> Self {
        Self {
            sink,
            dma: Vec::new(),
            peak_rgba_bytes: 0,
            peak_dma_bytes: 0,
        }
    }

    /// The underlying device sink.
    #[must_use]
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    /// Mutable access to the underlying device sink.
    pub const fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Largest RGBA band received so far, in bytes.
    #[must_use]
    pub const fn peak_rgba_bytes(&self) -> usize {
        self.peak_rgba_bytes
    }

    /// Largest RGB565 DMA band allocated so far, in bytes.
    #[must_use]
    pub const fn peak_dma_bytes(&self) -> usize {
        self.peak_dma_bytes
    }
}

impl<S: Rgb565Sink> DisplayFlush for Rgb565Display<S> {
    fn size(&self) -> (u32, u32) {
        self.sink.size()
    }

    fn flush_region(&mut self, region: DeviceRegion, pixels: &[u8]) {
        let (width, height) = self.sink.size();
        assert!(
            region.x + region.width <= width && region.y + region.height <= height,
            "RGB565 region {region:?} exceeds panel size {width}x{height}"
        );
        let pixel_count = usize::try_from(region.area()).expect("panel region area must fit usize");
        assert_eq!(
            pixels.len(),
            pixel_count * 4,
            "RGB565 source payload must match the flushed region"
        );
        let (rgba, remainder) = pixels.as_chunks::<4>();
        assert!(
            remainder.is_empty(),
            "RGBA payload must contain full pixels"
        );
        self.dma.resize(pixel_count, 0);
        for (target, source) in self.dma.iter_mut().zip(rgba) {
            *target = (u16::from(source[0]) >> 3) << 11
                | (u16::from(source[1]) >> 2) << 5
                | (u16::from(source[2]) >> 3);
        }
        self.peak_rgba_bytes = self.peak_rgba_bytes.max(pixels.len());
        self.peak_dma_bytes = self.peak_dma_bytes.max(pixel_count * 2);
        self.sink.write_region(region, &self.dma);
    }

    fn present(&mut self) {
        self.sink.present();
    }
}

/// Desktop/test display: a plain RGBA8 framebuffer in memory.
///
/// Used by the offscreen simulator and unit tests to assert on and
/// visually inspect rendered output. Embedded targets do not use this
/// type — it deliberately holds the full frame that dew otherwise avoids
/// allocating.
#[cfg(feature = "host")]
#[derive(Debug, Clone)]
pub struct BufferDisplay {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

#[cfg(feature = "host")]
impl BufferDisplay {
    /// Creates a black, fully transparent framebuffer.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The framebuffer contents as premultiplied RGBA8, row-major.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Encodes the framebuffer as a PNG (un-premultiplying alpha), for
    /// snapshot tests and the offscreen simulator.
    ///
    /// # Panics
    ///
    /// Panics when the framebuffer exceeds `u16::MAX` in either dimension
    /// or PNG encoding fails — both indicate misuse of a test-side type.
    #[must_use]
    pub fn to_png(&self) -> Vec<u8> {
        let width = u16::try_from(self.width).expect("framebuffer width exceeds u16::MAX");
        let height = u16::try_from(self.height).expect("framebuffer height exceeds u16::MAX");
        let mut pixmap = vello_cpu::Pixmap::new(width, height);
        for (target, source) in pixmap
            .data_mut()
            .iter_mut()
            .zip(self.pixels.as_chunks::<4>().0)
        {
            target.r = source[0];
            target.g = source[1];
            target.b = source[2];
            target.a = source[3];
        }
        pixmap
            .into_png()
            .expect("PNG encoding of framebuffer failed")
    }

    /// Returns the premultiplied RGBA8 value at (`x`, `y`).
    ///
    /// # Panics
    ///
    /// Panics when the coordinate is outside the framebuffer.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) outside {}x{} framebuffer",
            self.width,
            self.height
        );
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels[offset..offset + 4].try_into().unwrap()
    }
}

#[cfg(feature = "host")]
impl DisplayFlush for BufferDisplay {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn flush_region(&mut self, region: DeviceRegion, pixels: &[u8]) {
        assert_eq!(
            pixels.len() as u64,
            region.area() * 4,
            "region pixel payload does not match region size"
        );
        let row_bytes = region.width as usize * 4;
        for row in 0..region.height as usize {
            let src = &pixels[row * row_bytes..(row + 1) * row_bytes];
            let dst_start =
                ((region.y as usize + row) * self.width as usize + region.x as usize) * 4;
            self.pixels[dst_start..dst_start + row_bytes].copy_from_slice(src);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct RecordingRgb565Sink {
        size: (u32, u32),
        region: Option<DeviceRegion>,
        pixels: Vec<u16>,
        presents: usize,
    }

    impl Rgb565Sink for RecordingRgb565Sink {
        fn size(&self) -> (u32, u32) {
            self.size
        }

        fn write_region(&mut self, region: DeviceRegion, pixels: &[u16]) {
            self.region = Some(region);
            self.pixels.clear();
            self.pixels.extend_from_slice(pixels);
        }

        fn present(&mut self) {
            self.presents += 1;
        }
    }

    #[cfg(feature = "host")]
    #[test]
    fn flushed_region_lands_at_its_offset() {
        let mut display = BufferDisplay::new(8, 4);
        let region = DeviceRegion {
            x: 2,
            y: 1,
            width: 3,
            height: 2,
        };
        let pixels: Vec<u8> = (0..region.area() * 4)
            .map(|i| u8::try_from(i % 256).unwrap())
            .collect();
        display.flush_region(region, &pixels);
        assert_eq!(display.pixel(2, 1), [0, 1, 2, 3]);
        assert_eq!(display.pixel(4, 2), [20, 21, 22, 23]);
        assert_eq!(display.pixel(0, 0), [0, 0, 0, 0]);
        assert_eq!(display.pixel(5, 1), [0, 0, 0, 0]);
    }

    #[test]
    fn rgb565_display_converts_one_reusable_band() {
        let region = DeviceRegion {
            x: 1,
            y: 0,
            width: 2,
            height: 1,
        };
        let mut display = Rgb565Display::new(RecordingRgb565Sink {
            size: (4, 2),
            region: None,
            pixels: Vec::new(),
            presents: 0,
        });

        display.flush_region(region, &[255, 0, 0, 255, 0, 255, 0, 255]);
        display.present();

        assert_eq!(display.sink().region, Some(region));
        assert_eq!(display.sink().pixels, [0xf800, 0x07e0]);
        assert_eq!(display.sink().presents, 1);
        assert_eq!(display.peak_rgba_bytes(), 8);
        assert_eq!(display.peak_dma_bytes(), 4);
    }
}
