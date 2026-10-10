//! PaperVault command line. Run `papervault --help` for usage.

mod web;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use papervault::tools::{self, Opts, Upload};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "papervault",
    version,
    about = "Private PDF tools that run entirely on your computer",
    long_about = "PaperVault edits and converts PDFs locally. Files never leave this machine.\n\n\
Examples:\n  papervault run merge a.pdf b.pdf -o out/\n  papervault run split big.pdf -p every=10 -o parts/\n  papervault run protect secret.pdf -p password=hunter2\n  papervault serve"
)]
struct Cli {
    /// Print machine-readable JSON instead of plain text (for scripts and AI agents)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a tool on one or more files
    Run {
        /// Tool id, e.g. merge, split, compress (see `papervault tools`)
        tool: String,
        /// Input files (PDFs, or images for images-to-pdf)
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Folder to write results into
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
        /// Tool option as key=value, repeatable (e.g. -p pages=1-3 -p angle=90)
        #[arg(short = 'p', long = "opt", value_name = "KEY=VALUE")]
        opts: Vec<String>,
    },
    /// List every tool with its options
    Tools,
    /// Open the browser-based UI on this machine (127.0.0.1 only)
    Serve {
        /// Port to listen on
        #[arg(long, default_value_t = 8844)]
        port: u16,
        /// Do not open a browser window automatically
        #[arg(long)]
        no_open: bool,
    },
}

fn parse_opts(pairs: &[String]) -> Result<Opts> {
    let mut opts = Opts::new();
    for pair in pairs {
        let (key, value) = pair
            .split_once('=')
            .with_context(|| format!("option '{pair}' must look like key=value"))?;
        opts.insert(key.trim().to_string(), value.to_string());
    }
    Ok(opts)
}

/// Pick a file name that does not overwrite anything already in `dir`.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .expect("an unused file name always exists")
}

fn run_tool(tool: &str, files: &[PathBuf], out: &Path, pairs: &[String], json_out: bool) -> Result<()> {
    let opts = parse_opts(pairs)?;
    let uploads: Vec<Upload> = files
        .iter()
        .map(|path| {
            let data = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(Upload { name, data })
        })
        .collect::<Result<_>>()?;

    let outputs = tools::run(tool, &uploads, &opts)?;

    fs::create_dir_all(out).with_context(|| format!("could not create {}", out.display()))?;
    let mut written = Vec::new();
    for output in &outputs {
        let path = unique_path(out, &output.name);
        fs::write(&path, &output.data).with_context(|| format!("could not write {}", path.display()))?;
        written.push(path);
    }

    if json_out {
        let listing = json!({
            "ok": true,
            "tool": tool,
            "outputs": written.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        });
        println!("{listing}");
    } else {
        for path in &written {
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

fn list_tools(json_out: bool) {
    if json_out {
        let list: Vec<_> = tools::TOOLS
            .iter()
            .map(|t| json!({ "id": t.id, "summary": t.summary, "input": t.input, "options": t.options }))
            .collect();
        println!("{}", json!({ "tools": list }));
        return;
    }
    for t in tools::TOOLS {
        println!("{:<16} {}", t.id, t.summary);
        println!("{:<16} input: {}", "", t.input);
        if !t.options.is_empty() {
            println!("{:<16} options: {}", "", t.options);
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { tool, files, out, opts } => {
            if let Err(err) = run_tool(&tool, &files, &out, &opts, cli.json) {
                if cli.json {
                    println!("{}", json!({ "ok": false, "error": format!("{err:#}") }));
                    std::process::exit(1);
                }
                bail!(err);
            }
            Ok(())
        }
        Command::Tools => {
            list_tools(cli.json);
            Ok(())
        }
        Command::Serve { port, no_open } => web::serve(port, !no_open),
    }
}
