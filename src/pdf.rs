//! Core PDF operations: merge, split, reorder, rotate, crop, metadata and text.
//!
//! Everything works on in-memory byte buffers. Nothing here touches the network.

use anyhow::{Context, Result, anyhow, bail};
use lopdf::{Dictionary, Document, Object, ObjectId};

/// Parse a PDF, refusing encrypted files (password-protected PDFs are not supported yet).
pub fn load(bytes: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(bytes).context("could not read this file as a PDF")?;
    if doc.is_encrypted() {
        bail!("this PDF is password-protected, which PaperVault does not support yet");
    }
    Ok(doc)
}

/// Serialise a document back to bytes.
pub fn save(doc: &mut Document) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    doc.save_to(&mut out).context("could not write PDF")?;
    Ok(out)
}

/// Page object ids in page order (page 1 first).
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.get_pages().into_values().collect()
}

/// The root of the page tree (the `/Pages` entry of the catalog).
pub fn pages_root(doc: &Document) -> Result<ObjectId> {
    let root = doc.trailer.get(b"Root").and_then(Object::as_reference)?;
    Ok(doc.get_dictionary(root)?.get(b"Pages").and_then(Object::as_reference)?)
}

/// Look up an attribute that may be inherited from an ancestor of the page tree.
pub fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut current = Some(page);
    for _ in 0..64 {
        let id = current?;
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(value.clone());
        }
        current = dict.get(b"Parent").and_then(Object::as_reference).ok();
    }
    None
}

/// Copy inherited page attributes onto each page, so pages can be moved safely
/// out of their original tree.
fn flatten_inheritance(doc: &mut Document) -> Result<()> {
    let keys: [&[u8]; 4] = [b"MediaBox", b"CropBox", b"Resources", b"Rotate"];
    for id in page_ids(doc) {
        for key in keys {
            if !doc.get_dictionary(id)?.has(key) {
                if let Some(value) = inherited(doc, id, key) {
                    doc.get_dictionary_mut(id)?.set(key, value);
                }
            }
        }
    }
    Ok(())
}

/// Add `name -> target` to a page's resource dictionary under `category` (e.g. `Font`).
/// Inherited resources are copied onto the page first so the page's other resources survive.
pub fn add_resource(doc: &mut Document, page: ObjectId, category: &[u8], name: &str, target: ObjectId) -> Result<()> {
    fn resolve(doc: &Document, value: &Object) -> Dictionary {
        match value {
            Object::Reference(id) => doc.get_dictionary(*id).cloned().unwrap_or_default(),
            Object::Dictionary(dict) => dict.clone(),
            _ => Dictionary::new(),
        }
    }
    let own = doc.get_dictionary(page)?.get(b"Resources").ok().cloned();
    let source = own.or_else(|| inherited(doc, page, b"Resources"));
    let mut resources = source.map(|v| resolve(doc, &v)).unwrap_or_default();
    let mut section = resources.get(category).map(|v| resolve(doc, v)).unwrap_or_default();
    section.set(name, Object::Reference(target));
    resources.set(category, Object::Dictionary(section));
    doc.get_dictionary_mut(page)?.set("Resources", Object::Dictionary(resources));
    Ok(())
}

/// Page size as `[x0, y0, x1, y1]` in points.
pub fn page_box(doc: &Document, page: ObjectId) -> [f32; 4] {
    let default = [0.0, 0.0, 612.0, 792.0];
    let Some(Object::Array(values)) = inherited(doc, page, b"MediaBox") else {
        return default;
    };
    let nums: Vec<f32> = values.iter().filter_map(|o| o.as_float().ok()).collect();
    match nums.as_slice() {
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => default,
    }
}

/// Parse a page list such as `1-3,5,8-` (1-based, `8-` means "to the end").
/// Each comma-separated item becomes its own group, which `split` relies on.
pub fn parse_groups(spec: &str, total: u32) -> Result<Vec<Vec<u32>>> {
    let mut groups = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (part, part),
        };
        let start: u32 = a.parse().with_context(|| format!("'{part}' is not a page number or range"))?;
        let end: u32 = if b.is_empty() {
            total
        } else {
            b.parse().with_context(|| format!("'{part}' is not a page number or range"))?
        };
        if start < 1 || end > total || start > end {
            bail!("page range '{part}' is outside 1-{total}");
        }
        groups.push((start..=end).collect());
    }
    if groups.is_empty() {
        bail!("no pages were specified");
    }
    Ok(groups)
}

/// Like [`parse_groups`] but flattened into one ordered list.
pub fn parse_pages(spec: &str, total: u32) -> Result<Vec<u32>> {
    Ok(parse_groups(spec, total)?.into_iter().flatten().collect())
}

/// Keep exactly the pages in `order` (1-based), in that order. Every other page is dropped.
pub fn arrange(mut doc: Document, order: &[u32]) -> Result<Document> {
    flatten_inheritance(&mut doc)?;
    let ids = page_ids(&doc);
    let chosen: Vec<ObjectId> = order
        .iter()
        .map(|n| {
            ids.get((*n as usize).wrapping_sub(1))
                .copied()
                .ok_or_else(|| anyhow!("page {n} does not exist (the document has {} pages)", ids.len()))
        })
        .collect::<Result<_>>()?;
    let root = pages_root(&doc)?;
    for id in &chosen {
        doc.get_dictionary_mut(*id)?.set("Parent", Object::Reference(root));
    }
    let root_dict = doc.get_dictionary_mut(root)?;
    root_dict.set("Kids", chosen.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>());
    root_dict.set("Count", chosen.len() as i64);
    doc.prune_objects();
    Ok(doc)
}

/// Concatenate PDFs in the order given.
pub fn merge(inputs: &[Vec<u8>]) -> Result<Vec<u8>> {
    let (first, rest) = inputs.split_first().ok_or_else(|| anyhow!("no files to merge"))?;
    let mut base = load(first)?;
    for bytes in rest {
        let mut other = load(bytes)?;
        // Shift object numbers so they cannot collide with the base document's objects.
        other.renumber_objects_with(base.max_id + 1);
        let other_pages = page_ids(&other);
        base.objects.extend(other.objects);
        base.max_id = base.objects.keys().map(|id| id.0).max().unwrap_or(base.max_id);
        append_pages(&mut base, &other_pages)?;
    }
    base.prune_objects();
    save(&mut base)
}

fn append_pages(doc: &mut Document, pages: &[ObjectId]) -> Result<()> {
    let root = pages_root(doc)?;
    for id in pages {
        doc.get_dictionary_mut(*id)?.set("Parent", Object::Reference(root));
    }
    {
        let kids = doc
            .get_dictionary_mut(root)?
            .get_mut(b"Kids")
            .and_then(Object::as_array_mut)
            .context("unsupported PDF: page tree has no direct Kids array")?;
        kids.extend(pages.iter().map(|id| Object::Reference(*id)));
    }
    let count = page_ids(doc).len() as i64;
    doc.get_dictionary_mut(root)?.set("Count", count);
    Ok(())
}

/// Split into one PDF per page group. Each group becomes its own file.
pub fn split(bytes: &[u8], groups: &[Vec<u32>]) -> Result<Vec<Vec<u8>>> {
    groups
        .iter()
        .map(|group| {
            let mut doc = arrange(load(bytes)?, group)?;
            save(&mut doc)
        })
        .collect()
}

/// Rotate the selected pages (all pages when `pages` is `None`) by a multiple of 90 degrees.
pub fn rotate(bytes: &[u8], angle: i64, selected: Option<&[u32]>) -> Result<Vec<u8>> {
    if angle % 90 != 0 {
        bail!("rotation must be a multiple of 90 degrees");
    }
    let mut doc = load(bytes)?;
    flatten_inheritance(&mut doc)?;
    for (index, id) in page_ids(&doc).into_iter().enumerate() {
        let number = index as u32 + 1;
        if selected.is_some_and(|s| !s.contains(&number)) {
            continue;
        }
        let dict = doc.get_dictionary_mut(id)?;
        let current = dict.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
        dict.set("Rotate", (current + angle).rem_euclid(360));
    }
    save(&mut doc)
}

/// Trim margins (points) from every page: `[top, right, bottom, left]`.
pub fn crop(bytes: &[u8], margins: [f32; 4]) -> Result<Vec<u8>> {
    let [top, right, bottom, left] = margins;
    let mut doc = load(bytes)?;
    for id in page_ids(&doc) {
        let [x0, y0, x1, y1] = page_box(&doc, id);
        let crop_box = vec![
            Object::Real(x0 + left),
            Object::Real(y0 + bottom),
            Object::Real(x1 - right),
            Object::Real(y1 - top),
        ];
        doc.get_dictionary_mut(id)?.set("CropBox", crop_box);
    }
    save(&mut doc)
}

/// Read a value from the document's Info dictionary.
fn info_string(doc: &Document, key: &str) -> Option<String> {
    let info = doc.trailer.get(b"Info").and_then(Object::as_reference).ok()?;
    let value = doc.get_dictionary(info).ok()?.get(key.as_bytes()).ok()?;
    value.as_str().ok().map(|s| String::from_utf8_lossy(s).into_owned())
}

/// Set Info-dictionary fields (title, author, subject, keywords, creator). Empty values are skipped.
pub fn set_metadata(bytes: &[u8], fields: &[(&str, &str)]) -> Result<Vec<u8>> {
    let mut doc = load(bytes)?;
    let info = match doc.trailer.get(b"Info").and_then(Object::as_reference) {
        Ok(id) => id,
        Err(_) => {
            let id = doc.add_object(Dictionary::new());
            doc.trailer.set("Info", Object::Reference(id));
            id
        }
    };
    for (key, value) in fields {
        if !value.trim().is_empty() {
            doc.get_dictionary_mut(info)?.set(*key, Object::string_literal(value.trim()));
        }
    }
    save(&mut doc)
}

/// Human-readable summary: page count, version, metadata and encryption state.
pub fn info(bytes: &[u8]) -> Result<String> {
    let doc = Document::load_mem(bytes).context("could not read this file as a PDF")?;
    let mut lines = vec![format!("PDF version: {}", doc.version)];
    if doc.is_encrypted() {
        lines.push("Encrypted: yes (password required)".into());
        return Ok(lines.join("\n"));
    }
    lines.push("Encrypted: no".into());
    lines.push(format!("Pages: {}", page_ids(&doc).len()));
    for key in ["Title", "Author", "Subject", "Keywords", "Creator", "Producer"] {
        if let Some(value) = info_string(&doc, key) {
            lines.push(format!("{key}: {value}"));
        }
    }
    Ok(lines.join("\n"))
}

/// Extract text from the selected pages (all pages when `None`).
pub fn to_text(bytes: &[u8], selected: Option<&[u32]>) -> Result<String> {
    let doc = load(bytes)?;
    let numbers: Vec<u32> = match selected {
        Some(s) => s.to_vec(),
        None => doc.get_pages().keys().copied().collect(),
    };
    doc.extract_text(&numbers).context("could not extract text from this PDF")
}
