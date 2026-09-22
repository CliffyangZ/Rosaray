//! On-demand thumbnail generation (research.md §7, FR-047/FR-049).
//! Derived from the grayscale research representation, never eagerly
//! generated at import time, never a legitimate Run/measurement input.

use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};

pub const THUMBNAIL_MAX_DIM: u32 = 160;

pub fn render_thumbnail_png(source: &DynamicImage) -> Vec<u8> {
    let thumb = source.resize(THUMBNAIL_MAX_DIM, THUMBNAIL_MAX_DIM, FilterType::Triangle);
    let mut out = Vec::new();
    thumb
        .to_luma8()
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .expect("encoding an in-memory thumbnail buffer as PNG cannot fail");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_is_bounded_by_max_dimension() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            4000,
            2000,
            image::Rgb([1, 2, 3]),
        ));
        let png_bytes = render_thumbnail_png(&img);
        let decoded = image::load_from_memory(&png_bytes).unwrap();
        assert!(decoded.width() <= THUMBNAIL_MAX_DIM);
        assert!(decoded.height() <= THUMBNAIL_MAX_DIM);
    }
}
