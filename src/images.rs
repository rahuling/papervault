//! Image handling: images to PDF, compressing embedded photos, and extracting images from PDFs.

use anyhow::{Context, Result, bail};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, Rgba, RgbImage, RgbaImage};
use lopdf::{Document, Object, Stream, dictionary};
use std::io::Cursor;

/// Page size for images-to-PDF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSize {
    /// Each page is exactly the size of its image (96 dpi).
    Fit,
    A4,
    Letter,
}

impl PageSize {
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "" | "fit" => Ok(Self::Fit),
            "a4" => Ok(Self::A4),
            "letter" => Ok(Self::Letter),
            other => bail!("unknown page size '{other}' (use fit, a4 or letter)"),
        }
    }
}

fn encode_jpeg(image: DynamicImage, gray: bool, quality: u8) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    if gray {
        DynamicImage::from(image.to_luma8()).write_with_encoder(encoder)?;
    } else {
        DynamicImage::from(image.to_rgb8()).write_with_encoder(encoder)?;
    }
    Ok(buf)
}

/// Build a PDF with one image per page. JPEG input is embedded as-is (no re-encoding).
pub fn images_to_pdf(images: &[Vec<u8>], page_size: PageSize) -> Result<Vec<u8>> {
    if images.is_empty() {
        bail!("no images were provided");
    }
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.add_object(dictionary! {});
    let mut kids = Vec::new();

    for (index, raw) in images.iter().enumerate() {
        let image = image::load_from_memory(raw).with_context(|| format!("image {} is not a supported format", index + 1))?;
        let (width, height) = (image.width(), image.height());
        let is_jpeg = image::guess_format(raw).ok() == Some(ImageFormat::Jpeg);
        let gray = matches!(image.color(), image::ColorType::L8);

        // Pass JPEG straight through when it is plain RGB or gray; otherwise re-encode.
        let (data, gray) = if is_jpeg && matches!(image.color(), image::ColorType::L8 | image::ColorType::Rgb8) {
            (raw.clone(), gray)
        } else if image.color().has_alpha() {
            let mut base = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
            image::imageops::overlay(&mut base, &image.to_rgba8(), 0, 0);
            (encode_jpeg(DynamicImage::from(base), false, 92)?, false)
        } else {
            (encode_jpeg(image, gray, 92)?, gray)
        };

        let (page_w, page_h) = match page_size {
            PageSize::Fit => (width as f32 * 0.75, height as f32 * 0.75),
            PageSize::A4 => (595.0, 842.0),
            PageSize::Letter => (612.0, 792.0),
        };
        // Fit the image inside the page with a margin, centred.
        let margin = if page_size == PageSize::Fit { 0.0 } else { 24.0 };
        let scale = ((page_w - 2.0 * margin) / width as f32).min((page_h - 2.0 * margin) / height as f32);
        let (draw_w, draw_h) = (width as f32 * scale, height as f32 * scale);
        let (x, y) = ((page_w - draw_w) / 2.0, (page_h - draw_h) / 2.0);

        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => width as i64,
                "Height" => height as i64,
                "ColorSpace" => if gray { "DeviceGray" } else { "DeviceRGB" },
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            data,
        ));
        let content = format!("q {draw_w:.3} 0 0 {draw_h:.3} {x:.3} {y:.3} cm /Im0 Do Q");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(page_w), Object::Real(page_h)],
            "Contents" => content_id,
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Im0" => image_id },
            },
        });
        kids.push(Object::Reference(page_id));
    }

    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    doc.compress();
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

/// Downscale and re-encode JPEG images that are larger than `max_side` pixels, or simply large
/// in bytes. Only plain DeviceRGB / DeviceGray JPEGs are touched; anything else is left alone.
/// Returns how many images were replaced.
pub fn shrink_images(doc: &mut Document, max_side: u32, quality: u8) -> Result<usize> {
    let mut replaced = 0;
    for object in doc.objects.values_mut() {
        let Object::Stream(stream) = object else {
            continue;
        };
        let is_image = stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image".as_slice());
        let is_jpeg = stream.dict.get(b"Filter").and_then(Object::as_name).ok() == Some(b"DCTDecode".as_slice());
        if !is_image || !is_jpeg || stream.dict.has(b"SMask") {
            continue;
        }
        let gray = match stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok() {
            Some(b"DeviceGray") => true,
            Some(b"DeviceRGB") => false,
            _ => continue,
        };
        let Ok(image) = image::load_from_memory_with_format(&stream.content, ImageFormat::Jpeg) else {
            continue;
        };
        let (width, height) = (image.width(), image.height());
        let needs_resize = width.max(height) > max_side;
        let image = if needs_resize {
            image.resize(max_side, max_side, image::imageops::FilterType::Triangle)
        } else {
            image
        };
        let Ok(encoded) = encode_jpeg(image.clone(), gray, quality) else {
            continue;
        };
        // Keep the original unless the replacement is actually smaller.
        if !needs_resize && encoded.len() >= stream.content.len() {
            continue;
        }
        stream.dict.set("Width", image.width() as i64);
        stream.dict.set("Height", image.height() as i64);
        stream.set_content(encoded);
        replaced += 1;
    }
    Ok(replaced)
}

/// Extract embedded images. JPEG images are returned as `.jpg`; raw 8-bit RGB/gray images as `.png`.
pub fn extract_images(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let doc = Document::load_mem(bytes).context("could not read this file as a PDF")?;
    if doc.is_encrypted() {
        bail!("password-protected PDFs are not supported yet");
    }
    let mut out = Vec::new();
    for (id, object) in &doc.objects {
        let Object::Stream(stream) = object else {
            continue;
        };
        if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image".as_slice()) {
            continue;
        }
        let width = stream.dict.get(b"Width").and_then(Object::as_i64).unwrap_or(0) as usize;
        let height = stream.dict.get(b"Height").and_then(Object::as_i64).unwrap_or(0) as usize;
        let filter = stream.dict.get(b"Filter").and_then(Object::as_name).ok();
        if filter == Some(b"DCTDecode".as_slice()) {
            out.push((format!("image-{}.jpg", id.0), stream.content.clone()));
            continue;
        }
        let channels = match stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok() {
            Some(b"DeviceRGB") => 3,
            Some(b"DeviceGray") => 1,
            _ => continue,
        };
        let Ok(raw) = stream.decompressed_content() else {
            continue;
        };
        if width == 0 || height == 0 || raw.len() != width * height * channels {
            continue;
        }
        let dynamic = if channels == 3 {
            RgbImage::from_raw(width as u32, height as u32, raw).map(DynamicImage::from)
        } else {
            image::GrayImage::from_raw(width as u32, height as u32, raw).map(DynamicImage::from)
        };
        let Some(dynamic) = dynamic else {
            continue;
        };
        let mut png = Vec::new();
        dynamic.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
        out.push((format!("image-{}.png", id.0), png));
    }
    Ok(out)
}
