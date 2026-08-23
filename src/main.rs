use clap::Parser;
use onenote2rnote::onedata;
use onenote2rnote::rnote;
use onenote2rnote::rnote::{BackgroundKind, FormatKind, Options};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "onenote2rnote",
    about = "Convert OneNote handwritten notes to Rnote (.rnote), preserving vector strokes",
    version
)]
struct Cli {
    /// OneNote input: a .one section file, a .onetoc2 / .onepkg notebook, or a directory with .one files
    input: PathBuf,

    /// Output .rnote file (default: input path with .rnote extension)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Page size to use: a4, us_letter, or source (use the OneNote page height)
    #[arg(long, default_value = "source", value_parser = ["source", "a4", "us_letter"])]
    format: String,

    /// Rnote file-format version to write (must match your installed Rnote)
    #[arg(long, default_value = "0.15.0")]
    rnote_version: String,

    /// DPI of the produced Rnote document
    #[arg(long, default_value_t = 96.0)]
    dpi: f64,

    /// Page margin in px applied around the handwriting
    #[arg(long, default_value_t = 48.0)]
    margin: f64,

    /// Minimum page height in mm
    #[arg(long)]
    min_page_height_mm: Option<f64>,

    /// Background pattern: none, lines, or grid
    #[arg(long, default_value = "lines", value_parser = ["none", "lines", "grid"])]
    background: String,

    /// Do not shift and re-align the handwriting onto the page grid
    #[arg(long)]
    no_normalize: bool,

    /// Print a summary of found pages and strokes
    #[arg(long)]
    list_pages: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let options = Options {
        rnote_version: cli.rnote_version,
        dpi: cli.dpi,
        format: match cli.format.as_str() {
            "a4" => FormatKind::A4,
            "us_letter" => FormatKind::UsLetter,
            _ => FormatKind::Source,
        },
        min_page_height_mm: cli.min_page_height_mm,
        margin_px: cli.margin,
        background: match cli.background.as_str() {
            "none" => BackgroundKind::None,
            "grid" => BackgroundKind::Grid,
            _ => BackgroundKind::Lines,
        },
        normalize: !cli.no_normalize,
    };

    let pages = onedata::parse_input(&cli.input)?;

    if cli.list_pages || cli.verbose {
        let total: usize = pages.iter().map(|p| p.strokes.len()).sum();
        println!(
            "parsed {} pages, {} strokes total",
            pages.len(),
            total
        );
        for (i, page) in pages.iter().enumerate() {
            println!(
                "  page {:3}: {:4} strokes  {:?}",
                i + 1,
                page.strokes.len(),
                page.title.as_deref().unwrap_or("")
            );
        }
        if pages.iter().all(|p| !p.has_ink()) {
            eprintln!("warning: no handwritten ink found in the input.");
        }
    }

    let prepared = rnote::prepare_strokes(&pages, &options);
    let bytes = rnote::build_rnote_bytes(&prepared, &options)?;

    let output = cli
        .output
        .unwrap_or_else(|| default_output(&cli.input));

    std::fs::write(&output, &bytes)
        .map_err(|e| anyhow::anyhow!("writing {} failed: {e}", output.display()))?;

    let n_empty = prepared.iter().filter(|p| !p.has_ink()).count();
    if n_empty > 0 && cli.verbose {
        eprintln!(
            "note: {n_empty} of {} input pages had no handwriting and were skipped",
            prepared.len()
        );
    }

    println!(
        "wrote {} ({} stroke(s) on {} page(s))",
        output.display(),
        prepared.iter().map(|p| p.strokes.len()).sum::<usize>(),
        prepared.iter().filter(|p| p.has_ink()).count()
    );
    Ok(())
}

fn default_output(input: &Path) -> PathBuf {
    let mut out = input.as_os_str().to_owned();
    out.push(".rnote");
    PathBuf::from(out)
}
