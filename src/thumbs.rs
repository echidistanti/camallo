//! Small pictures for the list: fetched from the address yt-dlp gave, cut to 16:9 and shrunk.
use eframe::egui::ColorImage;
use image::{DynamicImage, GenericImageView};
use std::time::Duration;

/// Widest picture kept; the list shows them much smaller.
const WIDTH: u32 = 256;
const MAX_BYTES: u64 = 6 * 1024 * 1024;

pub fn fetch(url: &str) -> Option<ColorImage> {
    let cfg = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(12))).build();
    let agent: ureq::Agent = cfg.into();
    let mut r = agent.get(url).header("User-Agent", "Mozilla/5.0").call().ok()?;
    let bytes = r.body_mut().with_config().limit(MAX_BYTES).read_to_vec().ok()?;
    decode(&bytes)
}

/// JPEG, PNG or WebP bytes as a picture of at most WIDTH pixels, 16:9.
pub fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let img = crop_16_9(image::load_from_memory(bytes).ok()?);
    // Never enlarge: a small picture stays small.
    let img = if img.width() > WIDTH { img.thumbnail(WIDTH, WIDTH) } else { img };
    let (w, h) = img.dimensions();
    Some(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.to_rgba8().as_raw()))
}

/// Many sites give 4:3 pictures with black bars; cutting to 16:9 removes them.
fn crop_16_9(img: DynamicImage) -> DynamicImage {
    let (w, h) = img.dimensions();
    let want_h = ((w as u64 * 9 / 16) as u32).max(1);
    if want_h < h {
        return img.crop_imm(0, (h - want_h) / 2, w, want_h);
    }
    let want_w = ((h as u64 * 16 / 9) as u32).max(1);
    if want_w < w { img.crop_imm((w - want_w) / 2, 0, want_w, h) } else { img }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbImage};
    use std::io::Cursor;

    fn encoded(w: u32, h: u32, f: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(vec![]);
        DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, image::Rgb([200, 30, 90]))).write_to(&mut out, f).unwrap();
        out.into_inner()
    }

    #[test]
    fn pictures_in_the_usual_formats_come_out_16_9_and_small() {
        for f in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let img = decode(&encoded(480, 360, f)).unwrap_or_else(|| panic!("{f:?} did not decode"));
            assert_eq!(img.size, [256, 144], "{f:?}: 4:3 bars are cut, then shrunk to the width");
        }
        assert_eq!(decode(&encoded(1920, 1080, ImageFormat::Jpeg)).unwrap().size, [256, 144]);
    }

    #[test]
    fn small_and_wide_pictures_are_handled() {
        // Already smaller than the limit: not enlarged. Wider than 16:9: the sides are cut.
        assert_eq!(decode(&encoded(160, 90, ImageFormat::Png)).unwrap().size, [160, 90]);
        assert_eq!(decode(&encoded(400, 100, ImageFormat::Png)).unwrap().size, [177, 100]);
        assert!(decode(&encoded(1, 1, ImageFormat::Png)).is_some());
    }

    #[test]
    fn what_is_not_a_picture_is_refused() {
        assert!(decode(b"<html>403 Forbidden</html>").is_none());
        assert!(decode(&[]).is_none());
    }
}
