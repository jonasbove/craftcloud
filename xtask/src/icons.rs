//! `cargo xtask icons`: render the desktop client's packaging icons from the CraftCloud mark (the
//! mackerel), `design/assets/logo/craftcloud-logo-1024.png`. A new logo is this one command.
//!
//! Writes (all committed, so packaging and the Rust build never run it):
//! - `packaging/linux/icons/hicolor/<n>x<n>/apps/io.github.jonasbove.craftcloud.png`, 16 … 512
//! - `apps/craftcloud/assets/craftcloud.ico`, 16 … 256 as PNG frames; `apps/craftcloud/build.rs`
//!   puts it in craftcloud.exe and the MSI uses it for Apps & features
//! - `packaging/macos/craftcloud.icns`, 32 … 1024 as PNG entries, the mark on Apple's icon grid
//!   (824 of 1024 px, a transparent margin around it)
//!
//! The in-app PNGs (`apps/craftcloud/assets/craftcloud-logo-*.png`, `crates/ui-egui/assets/`) are
//! not touched: the window, tray and snapshot tests use those as they are.
//!
//! Resampling is Lanczos (a = 3) on premultiplied alpha, as Pillow's `resize` did for the shell
//! script this replaces, so transparent pixels don't bleed their colour into the edge.

use std::path::{Path, PathBuf};

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba, RgbaImage};

pub const USAGE: &str = "usage: cargo xtask icons [SOURCE.png]
  SOURCE   a square RGBA PNG of at least 512 px (default: design/assets/logo/craftcloud-logo-1024.png)";

const APP_ID: &str = "io.github.jonasbove.craftcloud";
const DEFAULT_SOURCE: &str = "design/assets/logo/craftcloud-logo-1024.png";
/// The smallest source that still gives a sharp 512 px icon.
const MIN_SOURCE: u32 = 512;
/// The largest source read, which bounds the f32 working copy (16 bytes a pixel, 1 GiB at 8192 px);
/// the icons go up to 1024 px.
const MAX_SOURCE: u32 = 8192;

/// Linux hicolor theme sizes.
pub const HICOLOR_SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];
/// Windows .ico frames: every size Explorer, the taskbar and Apps & features ask for.
pub const ICO_SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];
/// macOS .icns entries (all PNG): the type and its pixel size. `ic11` … `ic14` are the @2x
/// variants of 16, 32, 128 and 256.
pub const ICNS_ENTRIES: [(&[u8; 4], u32); 8] =
    [(b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512)];
/// Apple's icon grid: the shape is 824 of 1024 px, 100 px of transparent margin on each side.
const ICNS_CANVAS: u32 = 1024;
const ICNS_SHAPE: u32 = 824;

pub fn cmd(args: &[String], root: &Path) -> Result<(), String> {
    let source = match args {
        [] => root.join(DEFAULT_SOURCE),
        [one] if !one.starts_with('-') => PathBuf::from(one),
        _ => return Err(USAGE.to_owned()),
    };
    let mark = load_mark(&source)?;
    for (rel, bytes) in render(&mark)? {
        let out = root.join(&rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
        eprintln!("wrote {rel}");
    }
    println!("icons written from {}", source.display());
    Ok(())
}

/// The source mark as RGBA, checked to be square and large enough.
fn load_mark(path: &Path) -> Result<RgbaImage, String> {
    let reader = image::ImageReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let reader = reader.with_guessed_format().map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = reader.into_dimensions().map_err(|e| format!("{}: {e}", path.display()))?;
    check_size(path, w, h)?;
    let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(img.to_rgba8())
}

fn check_size(path: &Path, w: u32, h: u32) -> Result<(), String> {
    if w != h || !(MIN_SOURCE..=MAX_SOURCE).contains(&w) {
        return Err(format!("{} must be square and {MIN_SOURCE} to {MAX_SOURCE} px (is {w}x{h})", path.display()));
    }
    Ok(())
}

/// Every packaging icon for `mark`: (path relative to the repository root, file bytes).
fn render(mark: &RgbaImage) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = Vec::new();
    for n in HICOLOR_SIZES {
        files.push((format!("packaging/linux/icons/hicolor/{n}x{n}/apps/{APP_ID}.png"), png(&sized(mark, n))?));
    }
    files.push(("apps/craftcloud/assets/craftcloud.ico".to_owned(), ico(mark)?));
    files.push(("packaging/macos/craftcloud.icns".to_owned(), icns(mark)?));
    Ok(files)
}

/// `img` scaled to `n` x `n` with Lanczos on premultiplied alpha.
fn sized(img: &RgbaImage, n: u32) -> RgbaImage {
    if img.width() == n && img.height() == n {
        return img.clone();
    }
    let premultiplied: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
        let [r, g, b, a] = img.get_pixel(x, y).0.map(|c| f32::from(c) / 255.0);
        Rgba([r * a, g * a, b * a, a])
    });
    let scaled = imageops::resize(&premultiplied, n, n, FilterType::Lanczos3);
    ImageBuffer::from_fn(n, n, |x, y| {
        let [r, g, b, a] = scaled.get_pixel(x, y).0;
        let a = a.clamp(0.0, 1.0);
        let straight = |c: f32| if a > 0.0 { (c / a).clamp(0.0, 1.0) } else { 0.0 };
        Rgba([to_u8(straight(r)), to_u8(straight(g)), to_u8(straight(b)), to_u8(a)])
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 0 ..= 255 first
fn to_u8(c: f32) -> u8 {
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// `img` as a PNG at the best compression.
fn png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, PngFilter::Adaptive)
        .write_image(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG: {e}"))?;
    Ok(out)
}

/// The Windows icon: one PNG frame per size in [`ICO_SIZES`].
fn ico(mark: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut frames = Vec::new();
    for n in ICO_SIZES {
        let frame = IcoFrame::with_encoded(png(&sized(mark, n))?, n, n, ExtendedColorType::Rgba8).map_err(|e| format!("ICO frame {n}: {e}"))?;
        frames.push(frame);
    }
    let mut out = Vec::new();
    IcoEncoder::new(&mut out).encode_images(&frames).map_err(|e| format!("ICO: {e}"))?;
    Ok(out)
}

/// The macOS icon: the mark on Apple's grid, a table of contents, then one PNG per entry in
/// [`ICNS_ENTRIES`]. Every length is big-endian and counts its own 8-byte header.
fn icns(mark: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut canvas = RgbaImage::new(ICNS_CANVAS, ICNS_CANVAS);
    let margin = i64::from((ICNS_CANVAS - ICNS_SHAPE) / 2);
    imageops::replace(&mut canvas, &sized(mark, ICNS_SHAPE), margin, margin);
    let mut entries = Vec::new();
    for (kind, n) in ICNS_ENTRIES {
        entries.push((kind, png(&sized(&canvas, n))?));
    }
    let len = |n: usize| u32::try_from(n + 8).map_err(|_| "ICNS: too large".to_owned());
    let mut toc = Vec::new();
    for (kind, data) in &entries {
        toc.extend_from_slice(*kind);
        toc.extend_from_slice(&len(data.len())?.to_be_bytes());
    }
    let mut body = Vec::new();
    body.extend_from_slice(b"TOC ");
    body.extend_from_slice(&len(toc.len())?.to_be_bytes());
    body.extend_from_slice(&toc);
    for (kind, data) in &entries {
        body.extend_from_slice(*kind);
        body.extend_from_slice(&len(data.len())?.to_be_bytes());
        body.extend_from_slice(data);
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&len(body.len())?.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 512 px test mark: an opaque red disc on a transparent square.
    fn mark() -> RgbaImage {
        RgbaImage::from_fn(512, 512, |x, y| {
            let (dx, dy) = (f64::from(x) - 255.5, f64::from(y) - 255.5);
            if dx * dx + dy * dy < 200.0 * 200.0 { Rgba([255, 0, 0, 255]) } else { Rgba([0, 0, 0, 0]) }
        })
    }

    fn png_size(data: &[u8]) -> (u32, u32) {
        assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");
        let img = image::load_from_memory_with_format(data, image::ImageFormat::Png).unwrap();
        assert_eq!(img.color(), image::ColorType::Rgba8);
        (img.width(), img.height())
    }

    fn be32(b: &[u8]) -> usize {
        u32::from_be_bytes(b[..4].try_into().unwrap()) as usize
    }

    #[test]
    fn the_size_lists_cover_what_each_platform_asks_for() {
        assert_eq!(HICOLOR_SIZES, [16, 24, 32, 48, 64, 128, 256, 512]);
        assert_eq!(ICO_SIZES, [16, 20, 24, 32, 40, 48, 64, 128, 256]);
        let icns: Vec<(&[u8], u32)> = ICNS_ENTRIES.iter().map(|(k, n)| (&k[..], *n)).collect();
        assert_eq!(
            icns,
            [(&b"ic07"[..], 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512)]
        );
    }

    #[test]
    fn render_writes_every_packaging_icon() {
        let files = render(&mark()).unwrap();
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        let mut want: Vec<String> = HICOLOR_SIZES.iter().map(|n| format!("packaging/linux/icons/hicolor/{n}x{n}/apps/{APP_ID}.png")).collect();
        want.push("apps/craftcloud/assets/craftcloud.ico".into());
        want.push("packaging/macos/craftcloud.icns".into());
        assert_eq!(paths, want);
        for ((_, data), n) in files.iter().zip(HICOLOR_SIZES) {
            assert_eq!(png_size(data), (n, n));
        }
    }

    #[test]
    fn the_ico_holds_one_png_frame_per_size() {
        let data = ico(&mark()).unwrap();
        assert_eq!(&data[..4], [0, 0, 1, 0]); // reserved, type 1 = icon
        let count = usize::from(u16::from_le_bytes([data[4], data[5]]));
        assert_eq!(count, ICO_SIZES.len());
        for (i, n) in ICO_SIZES.into_iter().enumerate() {
            let e = &data[6 + 16 * i..22 + 16 * i];
            // A width or height byte of 0 means 256.
            let side = |b: u8| if b == 0 { 256 } else { u32::from(b) };
            assert_eq!((side(e[0]), side(e[1])), (n, n));
            let size = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
            assert_eq!(png_size(&data[offset..offset + size]), (n, n));
        }
        // The image crate reads it back as an icon.
        assert_eq!(image::guess_format(&data).unwrap(), image::ImageFormat::Ico);
    }

    #[test]
    fn the_icns_has_a_toc_and_one_png_per_entry() {
        let data = icns(&mark()).unwrap();
        assert_eq!(&data[..4], b"icns");
        assert_eq!(be32(&data[4..]), data.len());
        let mut at = 8;
        let mut seen = Vec::new();
        let mut toc = Vec::new();
        while at < data.len() {
            let kind = &data[at..at + 4];
            let len = be32(&data[at + 4..]);
            let body = &data[at + 8..at + len];
            if kind == b"TOC " {
                toc = body.chunks(8).map(|c| (c[..4].to_vec(), be32(&c[4..]))).collect();
            } else {
                let n = ICNS_ENTRIES.iter().find(|(k, _)| &k[..] == kind).map(|(_, n)| *n).unwrap();
                assert_eq!(png_size(body), (n, n));
                seen.push((kind.to_vec(), len));
            }
            at += len;
        }
        assert_eq!(at, data.len());
        assert_eq!(toc, seen);
        let kinds: Vec<&[u8]> = seen.iter().map(|(k, _)| k.as_slice()).collect();
        let want: Vec<&[u8]> = ICNS_ENTRIES.iter().map(|(k, _)| &k[..]).collect();
        assert_eq!(kinds, want);
    }

    #[test]
    fn the_icns_leaves_apples_margin_around_the_mark() {
        let data = icns(&mark()).unwrap();
        let mut at = 8;
        while &data[at..at + 4] != b"ic10" {
            at += be32(&data[at + 4..]);
        }
        let img = image::load_from_memory(&data[at + 8..at + be32(&data[at + 4..])]).unwrap().to_rgba8();
        // The disc spans 400 of 512 px of the mark, so 400 * 824 / 512 ≈ 644 px around the centre:
        // the 100 px border stays clear and the centre is opaque red.
        for i in 0..1024 {
            for (x, y) in [(i, 0), (i, 99), (0, i), (99, i), (i, 924), (924, i)] {
                assert_eq!(img.get_pixel(x, y).0[3], 0, "({x}, {y}) is in the margin");
            }
        }
        assert_eq!(img.get_pixel(512, 512).0, [255, 0, 0, 255]);
    }

    #[test]
    fn scaling_keeps_transparent_pixels_from_darkening_the_edge() {
        // Straight-alpha resampling mixes the transparent black into the red edge and darkens it;
        // premultiplied keeps every visible edge pixel pure red.
        let small = sized(&mark(), 16);
        for p in small.pixels() {
            if p.0[3] > 0 {
                assert_eq!(&p.0[..3], [255, 0, 0], "{p:?}");
            }
        }
    }

    #[test]
    fn a_source_that_is_not_square_or_too_small_is_refused() {
        let p = Path::new("logo.png");
        assert!(check_size(p, 1024, 1024).is_ok());
        assert!(check_size(p, 512, 512).is_ok());
        assert!(check_size(p, 1024, 1000).is_err());
        assert!(check_size(p, 256, 256).is_err());
        assert!(check_size(p, 8193, 8193).is_err());
    }
}
