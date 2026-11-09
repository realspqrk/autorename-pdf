//! Page rendering for OCR and vision (pixel-identical to pypdfium2 `page.render(scale)`).

use pdfium_render::prelude::*;

/// Single render scale for OCR and vision (144 DPI). Intentional change vs 3.x
/// (OCR-only used 1.5): 2.0 recovers small glyphs such as thousands separators,
/// and one render serves both tiers.
pub const RENDER_SCALE: f32 = 2.0;

/// Target pixel size for a page: `ceil(points · scale)` per axis.
pub fn target_size(width_pt: f32, height_pt: f32, scale: f32) -> (u32, u32) {
    (
        (width_pt * scale).ceil() as u32,
        (height_pt * scale).ceil() as u32,
    )
}

/// Render to RGB8 on white with annotations and form fields drawn.
pub fn render_page(page: &PdfPage, scale: f32) -> Result<image::RgbImage, PdfiumError> {
    let (w, h) = target_size(page.width().value, page.height().value, scale);
    let cfg = PdfRenderConfig::new()
        .set_target_width(w as i32)
        .set_target_height(h as i32)
        .render_annotations(true)
        .render_form_data(true)
        .set_clear_color(PdfColor::WHITE);
    let img = page.render_with_config(&cfg)?.as_image()?;
    Ok(img.to_rgb8())
}

/// PNG for upload. `image`'s default (fast, no adaptive filtering) is ~1.6× PIL's
/// size; Best + Adaptive lands 10–15 % below PIL.
pub fn encode_png(img: &image::RgbImage) -> Result<Vec<u8>, image::ImageError> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_sizes() {
        assert_eq!(target_size(595.28, 841.89, 2.0), (1191, 1684));
        assert_eq!(target_size(595.28, 841.89, 1.5), (893, 1263));
    }
}
