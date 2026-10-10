---
name: papervault
description: Edit, convert and inspect PDF files locally with the PaperVault CLI (merge, split, extract or delete pages, reorder, rotate, crop, compress, watermark, page numbers, metadata, extract text or images, PDF to images, images to PDF). Use when the user wants to work with PDF files and must keep the files on their machine. No data leaves the computer.
---

# PaperVault: local PDF tools

PaperVault is a command-line program (`papervault`) that runs PDF operations on this machine. It makes no network calls. Use it instead of online PDF services whenever the user's files are private.

## Check that it is installed

```bash
papervault --version
```

If the command is missing, build it from the repository (requires Rust):

```bash
cargo install --path .
```

## Discover tools and their options

```bash
papervault tools --json
```

This prints every tool id, its input type, and its options as JSON. Always check it before guessing option names.

## Run a tool

```bash
papervault run <tool> <file...> -o <output-folder> [-p key=value ...] --json
```

- `--json` makes stdout a single JSON object: `{"ok": true, "tool": "...", "outputs": ["..."]}`. On failure it prints `{"ok": false, "error": "..."}` and exits with status 1. Parse this rather than the human text.
- `-o` is the folder to write results into (created if missing). Existing files are never overwritten; a `-2` suffix is added.
- `-p` (or `--opt`) passes one option per flag, e.g. `-p pages=1-3 -p angle=90`.
- Page lists are 1-based: `1,3-5,8-` (`8-` means "to the last page"). Ranges outside the document are rejected.

## Tools

| Tool | Input | Options |
|---|---|---|
| `merge` | 2+ PDFs | none. Output `merged.pdf` in the order given. |
| `split` | 1 PDF | `ranges=1-3,4-6` (one file per comma item) or `every=N` |
| `extract` | 1 PDF | `pages=1,3-5` (kept in the order listed) |
| `delete` | 1 PDF | `pages=2,7-9` |
| `reorder` | 1 PDF | `order=3,1,2` (unlisted pages are removed) |
| `rotate` | 1 PDF | `angle=90\|180\|270`, optional `pages=` |
| `crop` | 1 PDF | `top=`, `right=`, `bottom=`, `left=` in points |
| `compress` | 1 PDF | `max_side=1600`, `quality=70` (downscales embedded JPEG photos) |
| `watermark` | 1 PDF | `text=...` (required), `size=48`, `opacity=0.2`, `angle=45`, optional `pages=` |
| `page-numbers` | 1 PDF | `template="{n} of {total}"`, `position=bottom-center` (also top/bottom left/right), `start=1`, `size=10`, optional `pages=` |
| `metadata` | 1 PDF | `title=`, `author=`, `subject=`, `keywords=` |
| `info` | 1 PDF | none. Writes a `-info.txt` summary (pages, metadata). |
| `to-text` | 1 PDF | optional `pages=`. Writes a `.txt` file. |
| `extract-images` | 1 PDF | none. Writes `.jpg` / `.png` files. |
| `pdf-to-images` | 1 PDF | `format=png\|jpg`, `dpi=150`. Requires Poppler's `pdftoppm` on PATH. |
| `images-to-pdf` | 1+ images | `page_size=fit\|a4\|letter` |

## Examples

```bash
papervault run merge contract.pdf appendix.pdf -o out --json
papervault run split report.pdf -p every=10 -o out/parts --json
papervault run extract report.pdf -p pages=1,4-6 -o out --json
papervault run compress scan.pdf -p quality=60 -o out --json
papervault run watermark draft.pdf -p text=DRAFT -o out --json
papervault run to-text invoice.pdf -o out --json
papervault run images-to-pdf a.jpg b.png -p page_size=a4 -o out --json
```

## Limits to tell the user about

- Password-protected PDFs cannot be opened yet. Encryption (protect and unlock) is planned for a later release.
- Not available yet: OCR, PDF to Word/Excel, Office to PDF, e-signatures, redaction, and PDF/A conversion.
- `pdf-to-images` needs Poppler installed separately. The other tools need nothing else.
- Text extraction quality depends on how the PDF was made. Scanned PDFs have no text layer.

## Working guidance

1. Confirm the input file paths exist before running.
2. Write results to a new folder rather than next to the originals, unless the user asks otherwise.
3. Read the `outputs` array from the JSON response and report those paths to the user.
4. If a command fails, show the `error` message. Do not retry with the same options.
