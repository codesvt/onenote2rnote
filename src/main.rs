use clap::Parser;
use onenote2rnote::manifest::{Manifest, fingerprint_page};
use onenote2rnote::onedata::{MediaKind, PageData};
use onenote2rnote::rnote::{BackgroundKind, FormatKind, Options};
use onenote2rnote::{manifest, onedata, rnote};
use sanitize_filename::sanitize;
use std::collections::BTreeMap;
use std::ffi::OsStr;
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

    /// Write all pages into a single .rnote file (disables the default per-page output).
    /// Mutually exclusive with --out-dir.
    #[arg(short, long, conflicts_with = "out_dir")]
    output: Option<PathBuf>,

    /// Directory for one .rnote per page, together with the original media files.
    /// This is the default behaviour; --out-dir only changes the target directory.
    /// Also enables incremental updates via a manifest.
    #[arg(long)]
    out_dir: Option<PathBuf>,

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
    #[arg(long, default_value = "grid", value_parser = ["none", "lines", "grid"])]
    background: String,

    /// Do not shift and re-align the handwriting onto the page grid
    #[arg(long)]
    no_normalize: bool,

    /// Keep all content exactly at its original OneNote coordinates (no margins, no re-alignment).
    /// Implied in --out-dir mode.
    #[arg(long)]
    original_pos: bool,

    /// In --out-dir mode: remove manifest entries and output files for pages no longer present.
    #[arg(long)]
    prune: bool,

    /// Print a summary of found pages and strokes, then exit without writing any output
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
        normalize: !cli.no_normalize && !cli.original_pos,
        original_pos: cli.original_pos,
    };

    let pages = onedata::parse_input(&cli.input)?;

    if cli.list_pages || cli.verbose {
        let total: usize = pages.iter().map(|p| p.strokes.len()).sum();
        let total_media: usize = pages.iter().map(|p| p.media.len()).sum();
        println!("parsed {} pages, {} strokes, {} media items total", pages.len(), total, total_media);
        for (i, page) in pages.iter().enumerate() {
            println!(
                "  page {:3}: {:4} strokes, {:2} media  {:?}",
                i + 1,
                page.strokes.len(),
                page.media.len(),
                page.title.as_deref().unwrap_or("")
            );
        }
        if pages.iter().all(|p| !p.has_any_content()) {
            eprintln!("warning: no handwritten ink or media found in the input.");
        }
    }

    // `--list-pages` is a dry run: only print the summary, do not write any output.
    if cli.list_pages {
        return Ok(());
    }

    // Default is one .rnote per page. A single-file output only happens with an explicit `-o`.
    let out_dir = match (&cli.output, &cli.out_dir) {
        (Some(_), _) => None,
        (None, Some(dir)) => Some(dir.clone()),
        (None, None) => Some(default_out_dir(&cli.input)),
    };

    if let Some(out_dir) = &out_dir {
        export_to_dir(&pages, out_dir, &options, cli.prune, cli.verbose)?;
    } else {
        export_single_file(&pages, &options, cli.output.clone(), &cli.input)?;
    }

    Ok(())
}

/// Default per-page output directory: a folder next to the input, named after the input file.
fn default_out_dir(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(OsStr::to_str)
        .map(sanitize)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "rnote".to_string());
    input.parent().unwrap_or_else(|| Path::new(".")).join(stem)
}

fn export_single_file(
    pages: &[PageData],
    options: &Options,
    output: Option<PathBuf>,
    input: &Path,
) -> anyhow::Result<()> {
    let prepared = rnote::prepare_strokes(pages, options)?;
    let bytes = rnote::build_rnote_bytes(&prepared, options)?;

    let output = output.unwrap_or_else(|| default_output(input));
    std::fs::write(&output, &bytes)
        .map_err(|e| anyhow::anyhow!("writing {} failed: {e}", output.display()))?;

    let n_pages = prepared.iter().filter(|p| p.has_content()).count();
    println!(
        "wrote {} ({} item(s) on {} page(s))",
        output.display(),
        prepared.iter().map(|p| p.strokes.len() + p.images.len()).sum::<usize>(),
        n_pages
    );
    Ok(())
}

fn export_to_dir(
    pages: &[PageData],
    out_dir: &Path,
    options: &Options,
    prune: bool,
    verbose: bool,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)?;
    let manifest_path = out_dir.join(MANIFEST_NAME);
    let mut manifest = Manifest::load(&manifest_path);

    // Assign a zero-padded, sequentially numbered filename per page (e.g. "01 Formelsammlungen.rnote")
    // so the files are sorted by page order in the file manager.
    let names: Vec<(String, String)> = {
        let width = pages.len().to_string().len().max(2);
        pages
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let title = p
                    .title
                    .as_deref()
                    .map(sanitize)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Seite".to_string());
                let stem = format!("{:0width$} {}", i + 1, title);
                (stem.clone(), format!("{stem}.rnote"))
            })
            .collect()
    };

    let mut changed = 0usize;
    let mut unchanged = 0usize;
    let mut orphaned = manifest.orphaned(pages.iter().filter_map(|p| p.guid.as_deref()));

    // A change of the tool's output format invalidates all previously exported pages.
    let force_reexport = manifest.version != MANIFEST_VERSION;

    let mut new_entries: BTreeMap<String, manifest::PageEntry> = BTreeMap::new();

    for (page, (stem, rnote_name)) in pages.iter().zip(names.iter()) {
        let Some(guid) = page.guid.as_deref() else {
            // No GUID: can't do incremental; export anyway.
            export_page(page, stem, out_dir, options)?;
            changed += 1;
            continue;
        };

        let fp = fingerprint_page(page);
        let already = !force_reexport
            && manifest.pages.get(guid).map(|e| e.fingerprint.as_str()) == Some(fp.as_str());

        if already {
            unchanged += 1;
            if let Some(entry) = manifest.pages.get(guid) {
                new_entries.insert(guid.to_string(), entry.clone());
            }
            continue;
        }

        let mut produced = export_page(page, stem, out_dir, options)?;
        produced.push(rnote_name.clone());
        // Remove stale output files (e.g. the old per-page PDF sidecars) that are no longer produced.
        if let Some(old) = manifest.pages.get(guid) {
            for f in &old.filenames {
                if !produced.contains(f) {
                    let p = out_dir.join(f);
                    if p.exists() {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
        }
        changed += 1;
        new_entries.insert(guid.to_string(), manifest::PageEntry {
            fingerprint: fp,
            filenames: produced,
        });
    }

    // Orphaned pages: keep files unless --prune is set.
    if prune {
        let mut removed = 0usize;
        for guid in &orphaned {
            if let Some(entry) = manifest.pages.get(guid) {
                for f in &entry.filenames {
                    let p = out_dir.join(f);
                    if p.exists() {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
            removed += 1;
        }
        orphaned.clear();
        if verbose && removed > 0 {
            eprintln!("pruned {removed} orphaned page(s)");
        }
    }

    manifest.pages = new_entries;
    manifest.version = MANIFEST_VERSION;
    manifest.save(&manifest_path)?;

    println!(
        "{}: {changed} new/changed, {unchanged} unchanged, {} orphaned (out-dir {} page(s) written)",
        out_dir.display(),
        orphaned.len(),
        pages.len()
    );
    Ok(())
}

fn export_page(page: &PageData, stem: &str, out_dir: &Path, options: &Options) -> anyhow::Result<Vec<String>> {
    let prepared = rnote::prepare_page(page, options)?;
    let bytes = rnote::build_rnote_bytes_single(&prepared, options)?;
    let rnote_path = out_dir.join(format!("{stem}.rnote"));
    std::fs::write(&rnote_path, &bytes)
        .map_err(|e| anyhow::anyhow!("writing {} failed: {e}", rnote_path.display()))?;

    let mut produced = Vec::new();
    let mut written_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // OneNote stores an inserted multi-page PDF as one single-page PDF per page.
    // Merge them into a single original PDF for the page.
    let pdfs: Vec<&[u8]> = page
        .media
        .iter()
        .filter(|m| m.kind == MediaKind::Pdf)
        .map(|m| m.bytes.as_slice())
        .collect();
    if !pdfs.is_empty() {
        let merged = onenote2rnote::pdf::merge_pdfs(&pdfs)?;
        let name = format!("{stem}-original.pdf");
        std::fs::write(out_dir.join(&name), &merged)
            .map_err(|e| anyhow::anyhow!("writing {} failed: {e}", name))?;
        produced.push(name.clone());
        written_names.insert(name);
    }

    for (i, media) in page.media.iter().enumerate() {
        if media.kind == MediaKind::Pdf {
            continue; // handled above as part of the merged original
        }
        let ext = detect_media_ext(&media.bytes, media.kind);
        let name = format!("{stem}-media-{}.{}", i + 1, ext);
        if !written_names.insert(name.clone()) {
            continue;
        }
        std::fs::write(out_dir.join(&name), &media.bytes)
            .map_err(|e| anyhow::anyhow!("writing {} failed: {e}", name))?;
        produced.push(name);
    }

    Ok(produced)
}

/// Detect the file extension from the binary content (fallback per kind).
fn detect_media_ext(bytes: &[u8], kind: MediaKind) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "jpg"
    } else if bytes.starts_with(b"%PDF") {
        "pdf"
    } else {
        match kind {
            MediaKind::Image => "png",
            MediaKind::Pdf => "pdf",
            MediaKind::Other => "bin",
        }
    }
}

fn default_output(input: &Path) -> PathBuf {
    let mut out = input.as_os_str().to_owned();
    out.push(".rnote");
    PathBuf::from(out)
}

const MANIFEST_NAME: &str = ".onenote2rnote-manifest.json";
/// Bump to force re-export of all pages (e.g. when the output format changes).
const MANIFEST_VERSION: u32 = 2;
