//! The tool registry. Both the CLI and the web UI call [`run`], so every feature behaves the same everywhere.

use crate::{external, images, pdf, stamp};
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::io::{Cursor, Write};

/// Options as strings, e.g. `pages = "1-3,5"`. Keys match the CLI `--opt key=value` flags and the web form fields.
pub type Opts = HashMap<String, String>;

/// An input file.
pub struct Upload {
    pub name: String,
    pub data: Vec<u8>,
}

/// A produced file.
pub struct Output {
    pub name: String,
    pub data: Vec<u8>,
}

/// Static description of a tool, used for `papervault tools` and the help text.
pub struct ToolInfo {
    pub id: &'static str,
    pub summary: &'static str,
    pub input: &'static str,
    pub options: &'static str,
}

pub const TOOLS: &[ToolInfo] = &[
    ToolInfo { id: "merge", summary: "Combine PDFs into one, in the order given", input: "2+ PDFs", options: "" },
    ToolInfo { id: "split", summary: "Split a PDF into several files", input: "1 PDF", options: "ranges=1-3,4-6 | every=N" },
    ToolInfo { id: "extract", summary: "Keep only the pages you list, in that order", input: "1 PDF", options: "pages=1,3-5" },
    ToolInfo { id: "delete", summary: "Remove the pages you list", input: "1 PDF", options: "pages=2,7" },
    ToolInfo { id: "reorder", summary: "Rearrange pages (listed pages are kept in this order)", input: "1 PDF", options: "order=3,1,2" },
    ToolInfo { id: "rotate", summary: "Rotate pages by 90, 180 or 270 degrees", input: "1 PDF", options: "angle=90 [pages=1-3]" },
    ToolInfo { id: "crop", summary: "Trim margins from every page (points)", input: "1 PDF", options: "top=0 right=0 bottom=0 left=0" },
    ToolInfo { id: "compress", summary: "Shrink file size by downscaling embedded photos", input: "1 PDF", options: "max_side=1600 quality=70" },
    ToolInfo { id: "watermark", summary: "Stamp a diagonal text watermark", input: "1 PDF", options: "text=CONFIDENTIAL [size=48 opacity=0.2 angle=45 pages=]" },
    ToolInfo { id: "page-numbers", summary: "Add page numbers or header/footer text", input: "1 PDF", options: "template={n} of {total} [position=bottom-center start=1 size=10 pages=]" },
    ToolInfo { id: "metadata", summary: "Set title, author, subject and keywords", input: "1 PDF", options: "title= author= subject= keywords=" },
    ToolInfo { id: "info", summary: "Show page count, metadata and encryption state", input: "1 PDF", options: "" },
    ToolInfo { id: "to-text", summary: "Extract text to a .txt file", input: "1 PDF", options: "pages= (optional)" },
    ToolInfo { id: "extract-images", summary: "Save embedded images (JPG/PNG)", input: "1 PDF", options: "" },
    ToolInfo { id: "pdf-to-images", summary: "Render pages to PNG or JPG (needs Poppler)", input: "1 PDF", options: "format=png|jpg dpi=150" },
    ToolInfo { id: "images-to-pdf", summary: "Make a PDF with one image per page", input: "1+ images", options: "page_size=fit|a4|letter" },
];

fn opt<'a>(opts: &'a Opts, key: &str) -> Option<&'a str> {
    opts.get(key).map(|v| v.trim()).filter(|v| !v.is_empty())
}

fn required<'a>(opts: &'a Opts, key: &str) -> Result<&'a str> {
    opt(opts, key).ok_or_else(|| anyhow!("missing option '{key}'"))
}

fn number<T: std::str::FromStr>(opts: &Opts, key: &str, default: T) -> Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match opt(opts, key) {
        Some(v) => v.parse::<T>().with_context(|| format!("option '{key}' must be a number, got '{v}'")),
        None => Ok(default),
    }
}

fn one_file(files: &[Upload]) -> Result<&Upload> {
    match files {
        [file] => Ok(file),
        [] => bail!("this tool needs one file"),
        _ => bail!("this tool takes one file, got {}", files.len()),
    }
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

fn out(name: impl Into<String>, data: Vec<u8>) -> Vec<Output> {
    vec![Output { name: name.into(), data }]
}

/// Zip several outputs into one archive (used when a tool produces more than one file for the web UI).
pub fn zip(outputs: &[Output]) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for file in outputs {
            writer.start_file(file.name.as_str(), options)?;
            writer.write_all(&file.data)?;
        }
        writer.finish()?;
    }
    Ok(buf.into_inner())
}

/// Run a tool. Returns one or more output files.
pub fn run(tool: &str, files: &[Upload], opts: &Opts) -> Result<Vec<Output>> {
    match tool {
        "merge" => {
            if files.len() < 2 {
                bail!("merge needs at least two PDFs");
            }
            let docs: Vec<Vec<u8>> = files.iter().map(|f| f.data.clone()).collect();
            Ok(out("merged.pdf", pdf::merge(&docs)?))
        }
        "split" => {
            let file = one_file(files)?;
            let total = pdf::page_ids(&pdf::load(&file.data)?).len() as u32;
            let groups: Vec<Vec<u32>> = if let Some(every) = opt(opts, "every") {
                let n: u32 = every.parse().context("'every' must be a number")?;
                if n == 0 {
                    bail!("'every' must be at least 1");
                }
                (1..=total).collect::<Vec<_>>().chunks(n as usize).map(|c| c.to_vec()).collect()
            } else {
                pdf::parse_groups(required(opts, "ranges")?, total)?
            };
            let base = stem(&file.name);
            let parts = pdf::split(&file.data, &groups)?;
            Ok(parts
                .into_iter()
                .zip(&groups)
                .enumerate()
                .map(|(i, (data, group))| Output {
                    name: format!("{base}-part{}-p{}-{}.pdf", i + 1, group[0], group[group.len() - 1]),
                    data,
                })
                .collect())
        }
        "extract" => {
            let file = one_file(files)?;
            let total = pdf::page_ids(&pdf::load(&file.data)?).len() as u32;
            let pages = pdf::parse_pages(required(opts, "pages")?, total)?;
            let doc = pdf::arrange(pdf::load(&file.data)?, &pages)?;
            let mut doc = doc;
            Ok(out(format!("{}-extracted.pdf", stem(&file.name)), pdf::save(&mut doc)?))
        }
        "delete" => {
            let file = one_file(files)?;
            let doc = pdf::load(&file.data)?;
            let total = pdf::page_ids(&doc).len() as u32;
            let remove = pdf::parse_pages(required(opts, "pages")?, total)?;
            let keep: Vec<u32> = (1..=total).filter(|p| !remove.contains(p)).collect();
            if keep.is_empty() {
                bail!("that would delete every page");
            }
            let mut doc = pdf::arrange(doc, &keep)?;
            Ok(out(format!("{}-edited.pdf", stem(&file.name)), pdf::save(&mut doc)?))
        }
        "reorder" => {
            let file = one_file(files)?;
            let total = pdf::page_ids(&pdf::load(&file.data)?).len() as u32;
            let order = pdf::parse_pages(required(opts, "order")?, total)?;
            let mut doc = pdf::arrange(pdf::load(&file.data)?, &order)?;
            Ok(out(format!("{}-reordered.pdf", stem(&file.name)), pdf::save(&mut doc)?))
        }
        "rotate" => {
            let file = one_file(files)?;
            let angle: i64 = number(opts, "angle", 90)?;
            let selected = selected_pages(opts, &file.data)?;
            Ok(out(format!("{}-rotated.pdf", stem(&file.name)), pdf::rotate(&file.data, angle, selected.as_deref())?))
        }
        "crop" => {
            let file = one_file(files)?;
            let margins = [number(opts, "top", 0.0)?, number(opts, "right", 0.0)?, number(opts, "bottom", 0.0)?, number(opts, "left", 0.0)?];
            Ok(out(format!("{}-cropped.pdf", stem(&file.name)), pdf::crop(&file.data, margins)?))
        }
        "compress" => {
            let file = one_file(files)?;
            let max_side: u32 = number(opts, "max_side", 1600)?;
            let quality: u8 = number(opts, "quality", 70)?;
            let mut doc = pdf::load(&file.data)?;
            images::shrink_images(&mut doc, max_side, quality)?;
            doc.prune_objects();
            doc.compress();
            let mut data = pdf::save(&mut doc)?;
            // Keep the original if re-encoding did not help.
            if data.len() >= file.data.len() {
                data = file.data.clone();
            }
            Ok(out(format!("{}-compressed.pdf", stem(&file.name)), data))
        }
        "watermark" => {
            let file = one_file(files)?;
            let text = required(opts, "text")?;
            let size: f32 = number(opts, "size", 48.0)?;
            let opacity: f32 = number(opts, "opacity", 0.2)?;
            let angle: f32 = number(opts, "angle", 45.0)?;
            let selected = selected_pages(opts, &file.data)?;
            let data = stamp::watermark(&file.data, text, size, opacity, angle, selected.as_deref())?;
            Ok(out(format!("{}-watermarked.pdf", stem(&file.name)), data))
        }
        "page-numbers" => {
            let file = one_file(files)?;
            let template = opt(opts, "template").unwrap_or("{n}");
            let position = stamp::Position::parse(opt(opts, "position").unwrap_or(""))?;
            let start: u32 = number(opts, "start", 1)?;
            let size: f32 = number(opts, "size", 10.0)?;
            let selected = selected_pages(opts, &file.data)?;
            let data = stamp::page_numbers(&file.data, template, position, start, size, selected.as_deref())?;
            Ok(out(format!("{}-numbered.pdf", stem(&file.name)), data))
        }
        "metadata" => {
            let file = one_file(files)?;
            let fields = [
                ("Title", opt(opts, "title").unwrap_or("")),
                ("Author", opt(opts, "author").unwrap_or("")),
                ("Subject", opt(opts, "subject").unwrap_or("")),
                ("Keywords", opt(opts, "keywords").unwrap_or("")),
            ];
            Ok(out(format!("{}-metadata.pdf", stem(&file.name)), pdf::set_metadata(&file.data, &fields)?))
        }
        "info" => {
            let file = one_file(files)?;
            Ok(out(format!("{}-info.txt", stem(&file.name)), pdf::info(&file.data)?.into_bytes()))
        }
        "to-text" => {
            let file = one_file(files)?;
            let selected = selected_pages(opts, &file.data)?;
            Ok(out(format!("{}.txt", stem(&file.name)), pdf::to_text(&file.data, selected.as_deref())?.into_bytes()))
        }
        "extract-images" => {
            let file = one_file(files)?;
            let found = images::extract_images(&file.data)?;
            if found.is_empty() {
                bail!("no embedded images were found in this PDF");
            }
            Ok(found.into_iter().map(|(name, data)| Output { name, data }).collect())
        }
        "pdf-to-images" => {
            let file = one_file(files)?;
            let format = opt(opts, "format").unwrap_or("png");
            let dpi: u32 = number(opts, "dpi", 150)?;
            let base = stem(&file.name);
            Ok(external::pdf_to_images(&file.data, format, dpi)?
                .into_iter()
                .map(|(name, data)| Output { name: format!("{base}-{name}"), data })
                .collect())
        }
        "images-to-pdf" => {
            if files.is_empty() {
                bail!("choose at least one image");
            }
            let page_size = images::PageSize::parse(opt(opts, "page_size").unwrap_or("fit"))?;
            let data: Vec<Vec<u8>> = files.iter().map(|f| f.data.clone()).collect();
            Ok(out("images.pdf", images::images_to_pdf(&data, page_size)?))
        }
        other => bail!("unknown tool '{other}'. Run `papervault tools` to see the list"),
    }
}

/// Optional `pages` option, parsed against the file's page count. `None` means "all pages".
fn selected_pages(opts: &Opts, bytes: &[u8]) -> Result<Option<Vec<u32>>> {
    match opt(opts, "pages") {
        None => Ok(None),
        Some(spec) => {
            let total = pdf::page_ids(&pdf::load(bytes)?).len() as u32;
            Ok(Some(pdf::parse_pages(spec, total)?))
        }
    }
}
