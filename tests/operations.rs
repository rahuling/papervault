//! End-to-end checks for each operation, using PDFs and images generated in the test itself.

use image::{ImageFormat, RgbImage};
use lopdf::{Document, Object, Stream, dictionary};
use papervault::images::{self, PageSize};
use papervault::pdf;
use papervault::stamp::{self, Position};
use papervault::tools::{self, Opts, Upload};
use std::io::Cursor;

/// A PDF with `pages` pages, each containing the text "page N".
fn make_pdf(pages: u32) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.add_object(dictionary! {});
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let mut kids = Vec::new();
    for n in 1..=pages {
        let content = format!("BT /F1 24 Tf 72 720 Td (page {n}) Tj ET");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        kids.push(Object::Reference(page));
    }
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn page_count(bytes: &[u8]) -> usize {
    Document::load_mem(bytes).unwrap().get_pages().len()
}

fn upload(name: &str, data: Vec<u8>) -> Upload {
    Upload { name: name.into(), data }
}

fn opts(pairs: &[(&str, &str)]) -> Opts {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(width, height, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
    let mut out = Vec::new();
    image::DynamicImage::from(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
    out
}

#[test]
fn merge_concatenates_pages_in_order() {
    let merged = pdf::merge(&[make_pdf(2), make_pdf(3)]).unwrap();
    assert_eq!(page_count(&merged), 5);
    let text = pdf::to_text(&merged, None).unwrap();
    assert!(text.contains("page 1") && text.contains("page 3"));
}

#[test]
fn merge_requires_two_files() {
    let result = tools::run("merge", &[upload("a.pdf", make_pdf(1))], &Opts::new());
    assert!(result.is_err());
}

#[test]
fn split_by_ranges_makes_one_file_per_range() {
    let outputs = tools::run("split", &[upload("big.pdf", make_pdf(6))], &opts(&[("ranges", "1-2,3-6")])).unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(page_count(&outputs[0].data), 2);
    assert_eq!(page_count(&outputs[1].data), 4);
}

#[test]
fn split_every_n_pages() {
    let outputs = tools::run("split", &[upload("big.pdf", make_pdf(5))], &opts(&[("every", "2")])).unwrap();
    let counts: Vec<usize> = outputs.iter().map(|o| page_count(&o.data)).collect();
    assert_eq!(counts, vec![2, 2, 1]);
}

#[test]
fn extract_keeps_listed_pages_in_given_order() {
    let outputs = tools::run("extract", &[upload("a.pdf", make_pdf(4))], &opts(&[("pages", "3,1")])).unwrap();
    assert_eq!(page_count(&outputs[0].data), 2);
    let text = pdf::to_text(&outputs[0].data, None).unwrap();
    assert!(text.find("page 3").unwrap() < text.find("page 1").unwrap());
}

#[test]
fn delete_removes_listed_pages() {
    let outputs = tools::run("delete", &[upload("a.pdf", make_pdf(4))], &opts(&[("pages", "2-3")])).unwrap();
    assert_eq!(page_count(&outputs[0].data), 2);
}

#[test]
fn delete_rejects_removing_everything() {
    assert!(tools::run("delete", &[upload("a.pdf", make_pdf(2))], &opts(&[("pages", "1-2")])).is_err());
}

#[test]
fn rotate_sets_rotation_on_each_page() {
    let rotated = pdf::rotate(&make_pdf(2), 90, None).unwrap();
    let doc = Document::load_mem(&rotated).unwrap();
    for id in pdf::page_ids(&doc) {
        assert_eq!(doc.get_dictionary(id).unwrap().get(b"Rotate").unwrap().as_i64().unwrap(), 90);
    }
}

#[test]
fn rotate_rejects_non_right_angles() {
    assert!(pdf::rotate(&make_pdf(1), 45, None).is_err());
}

#[test]
fn crop_sets_crop_box() {
    let cropped = pdf::crop(&make_pdf(1), [10.0, 10.0, 10.0, 10.0]).unwrap();
    let doc = Document::load_mem(&cropped).unwrap();
    let page = pdf::page_ids(&doc)[0];
    let crop_box = doc.get_dictionary(page).unwrap().get(b"CropBox").unwrap().as_array().unwrap();
    assert_eq!(crop_box[0].as_float().unwrap(), 10.0);
    assert_eq!(crop_box[2].as_float().unwrap(), 602.0);
}

#[test]
fn watermark_adds_text_to_each_page() {
    let marked = stamp::watermark(&make_pdf(2), "DRAFT", 40.0, 0.2, 45.0, None).unwrap();
    assert_eq!(page_count(&marked), 2);
    assert!(pdf::to_text(&marked, None).unwrap().contains("DRAFT"));
}

#[test]
fn page_numbers_use_template() {
    let numbered = stamp::page_numbers(&make_pdf(3), "{n} of {total}", Position::BottomCenter, 1, 10.0, None).unwrap();
    let text = pdf::to_text(&numbered, None).unwrap();
    assert!(text.contains("2 of 3"));
}

#[test]
fn metadata_is_written_and_read_back() {
    let out = pdf::set_metadata(&make_pdf(1), &[("Title", "Quarterly report"), ("Author", "")]).unwrap();
    let info = pdf::info(&out).unwrap();
    assert!(info.contains("Title: Quarterly report"));
    assert!(info.contains("Pages: 1"));
}

#[test]
fn images_to_pdf_one_page_per_image() {
    let pdf_bytes = images::images_to_pdf(&[png(300, 200), png(100, 400)], PageSize::A4).unwrap();
    assert_eq!(page_count(&pdf_bytes), 2);
}

#[test]
fn images_round_trip_through_extract() {
    let pdf_bytes = images::images_to_pdf(&[png(64, 48)], PageSize::Fit).unwrap();
    let found = images::extract_images(&pdf_bytes).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].0.ends_with(".jpg"));
}

#[test]
fn compress_shrinks_large_photos() {
    // A large, noisy JPEG embedded in a PDF should get smaller after compression.
    let noisy = RgbImage::from_fn(2400, 1800, |x, y| image::Rgb([((x * 7 + y * 13) % 256) as u8, ((x ^ y) % 256) as u8, ((x * y) % 256) as u8]));
    let mut jpeg = Vec::new();
    image::DynamicImage::from(noisy).write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg).unwrap();
    let source = images::images_to_pdf(&[jpeg], PageSize::Fit).unwrap();
    let outputs = tools::run("compress", &[upload("photos.pdf", source.clone())], &Opts::new()).unwrap();
    assert!(outputs[0].data.len() < source.len(), "compressed {} vs original {}", outputs[0].data.len(), source.len());
    assert_eq!(page_count(&outputs[0].data), 1);
}

#[test]
fn unknown_tool_is_an_error() {
    assert!(tools::run("nope", &[], &Opts::new()).is_err());
}
