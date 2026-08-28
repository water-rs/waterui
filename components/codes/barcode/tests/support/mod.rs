use rxing::{
    BarcodeFormat, BinaryBitmap, DecodeHints, Luma8LuminanceSource, MultiFormatReader, Reader,
    common::HybridBinarizer,
};
use waterui_graphics::OffscreenRenderOutput;

pub fn decode(output: &OffscreenRenderOutput, format: BarcodeFormat) -> String {
    let luminance = output
        .rgba8
        .chunks_exact(4)
        .map(|rgba| {
            let [red, green, blue, _alpha] =
                <[u8; 4]>::try_from(rgba).expect("RGBA output must contain four-byte pixels");
            u8::try_from(
                (u32::from(red) * 2126 + u32::from(green) * 7152 + u32::from(blue) * 722) / 10_000,
            )
            .expect("weighted RGB luminance must fit into one byte")
        })
        .collect();
    let source = Luma8LuminanceSource::new(luminance, output.width, output.height)
        .expect("offscreen dimensions must match the pixel buffer");
    let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(source));
    let mut reader = MultiFormatReader::default();
    reader
        .decode_with_hints(
            &mut bitmap,
            &DecodeHints {
                PossibleFormats: Some([format].into()),
                TryHarder: Some(true),
                ..DecodeHints::default()
            },
        )
        .expect("GPU output must decode as the requested barcode format")
        .getText()
        .to_owned()
}
