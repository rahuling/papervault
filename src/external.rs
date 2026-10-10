//! Optional bridge to Poppler's `pdftoppm` for rasterising pages (PDF to PNG/JPG).
//!
//! Rendering PDF pages needs a full PDF renderer, which is not practical to write in pure Rust.
//! Poppler is free, local and widely packaged; if it is not installed the tool reports how to get it.

use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Render every page of a PDF to `png` or `jpg` at the given DPI. Returns `(file name, bytes)` pairs.
pub fn pdf_to_images(bytes: &[u8], format: &str, dpi: u32) -> Result<Vec<(String, Vec<u8>)>> {
    let (flag, ext) = match format.to_ascii_lowercase().as_str() {
        "png" => ("-png", "png"),
        "jpg" | "jpeg" => ("-jpeg", "jpg"),
        other => bail!("unknown image format '{other}' (use png or jpg)"),
    };
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let dir: PathBuf = std::env::temp_dir().join(format!("papervault-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).context("could not create a temporary folder")?;
    let result = render(&dir, bytes, flag, ext, dpi);
    let _ = fs::remove_dir_all(&dir);
    result
}

fn render(dir: &PathBuf, bytes: &[u8], flag: &str, ext: &str, dpi: u32) -> Result<Vec<(String, Vec<u8>)>> {
    let input = dir.join("input.pdf");
    fs::write(&input, bytes)?;
    let status = Command::new("pdftoppm")
        .arg(flag)
        .arg("-r")
        .arg(dpi.to_string())
        .arg(&input)
        .arg(dir.join("page"))
        .status()
        .map_err(|e| anyhow!("pdftoppm was not found ({e}). Install Poppler (Windows: https://github.com/oschwartz10612/poppler-windows/releases, macOS: brew install poppler, Linux: apt install poppler-utils) and put it on PATH"))?;
    if !status.success() {
        bail!("pdftoppm failed to render this PDF");
    }
    let mut names: Vec<String> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("page") && n.ends_with(ext))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let data = fs::read(dir.join(&name))?;
            Ok((name, data))
        })
        .collect()
}
