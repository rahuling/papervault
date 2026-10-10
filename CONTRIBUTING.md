# Contributing to PaperVault

Thanks for helping. PaperVault is a small project, so every kind of help counts: code, testing, design, documentation, and bug reports from real PDFs.

## Ground rules

- **Privacy comes first.** PaperVault must never send a user's files or usage data anywhere. Do not add network calls, analytics, remote fonts, or CDN scripts to the app.
- **Local and simple.** Prefer small, readable changes. If a feature needs a heavy external tool, make it optional and say so in the UI and README.
- **Tested.** Add or update an integration test in `tests/operations.rs` for any change to PDF behaviour.

## Set up

You need Rust 1.85 or newer.

```bash
git clone https://github.com/rahuling/papervault
cd papervault
cargo test          # all tests should pass
cargo run -- serve  # open the UI from source at http://127.0.0.1:8844
```

Check your change before opening a pull request:

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test
```

## Send a change

1. Fork the repository and create a branch with a short name, such as `fix-rotate-inherit`.
2. Make your change, with a test where it applies.
3. Open a pull request. Explain what changed and why, and say how you tested it. For UI changes, add a before and after screenshot.

Commit messages: a short imperative subject (for example, "Keep inherited Rotate when reordering"), and a body if the reason is not obvious.

## Good first contributions

- Reproduce a bug from the issue tracker and add a failing test for it.
- Add a missing page-number template token, or a clearer error message for a bad page range.
- Improve the CLI help text in `src/main.rs`.
- Test the build and the test suite on macOS or Linux, and report what you find.

## Design

PaperVault aims for the calm, precise feel of a professional document tool. Contributions to the logo, the screenshots, spacing, colour and interaction states are very welcome.

- Logo files live in `brand/` as vectors. Keep the mark readable at 16 px.
- The UI lives in `web/index.html` and uses system fonts and inline icons only.
- The public site lives in `docs/`. Screenshots are in `docs/screenshots/`.

If you change the interface, please attach updated screenshots so reviewers can see the difference.

## Test on your platform

PaperVault is developed on Windows. macOS and Linux builds should work with the same commands, but they have not been verified yet. If you have one of those systems:

1. Run `cargo test` and note any failures.
2. Run `cargo run -- serve` and check that the UI loads and a merge works.
3. Open an issue with your OS version, Rust version, and what happened.

PDF to images needs Poppler's `pdftoppm`. Test it separately if you can.

## Report a bug

Include the exact command or steps, the tool and options you used, the Rust and OS versions, and if you can share it, a small PDF that reproduces the problem. Do not attach private documents. If a file is sensitive, make a minimal example instead.

## Code of conduct

Be kind and specific. Assume good faith, and keep discussion focused on the work.
