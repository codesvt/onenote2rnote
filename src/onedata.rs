use onenote_parser::contents::{Ink, InkStroke};
use onenote_parser::notebook::Notebook;
use onenote_parser::page::{Page, PageContent};
use onenote_parser::section::{Section, SectionEntry};
use onenote_parser::Parser;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use typed_path::TypedPath;

#[derive(Debug, Clone)]
pub struct InkStrokeData {
    pub points: Vec<(f64, f64)>,
    pub width_ink: f64,
    pub color: Option<u32>,
    pub transparency: Option<u8>,
    pub off_half_inch: (f64, f64),
}

#[derive(Debug, Clone)]
pub struct PageData {
    pub strokes: Vec<InkStrokeData>,
    pub height_half_inch: Option<f64>,
    pub title: Option<String>,
}

impl PageData {
    pub fn has_ink(&self) -> bool {
        !self.strokes.is_empty()
    }
}

pub fn parse_input(path: &Path) -> anyhow::Result<Vec<PageData>> {
    let name_lower = path
        .file_name()
        .and_then(OsStr::to_str)
        .map(|n| n.to_ascii_lowercase());
    let ext = path.extension().and_then(OsStr::to_str).map(str::to_lowercase);

    let kind = match (name_lower.as_deref(), ext.as_deref()) {
        (Some(n), _) if n.ends_with(".onepkg") => Some("onepkg"),
        (Some(n), _) if n.ends_with(".onetoc2") => Some("onetoc2"),
        (_, Some("one")) => Some("one"),
        _ => None,
    };

    let Some(kind) = kind else {
        anyhow::bail!(
            "unsupported file type {:?} (expected .one, .onetoc2, .onepkg or a directory)",
            path
        );
    };
    let mut pages: Vec<PageData> = Vec::new();

    if path.is_dir() {
        let mut files: Vec<_> = fs::read_dir(path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(OsStr::to_str)
                    .map(|e| e.eq_ignore_ascii_case("one"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        if files.is_empty() {
            anyhow::bail!("no .one files found in directory {:?}", path);
        }
        for f in files {
            pages.extend(section_pages(&f)?);
        }
        return Ok(pages);
    }

    match kind {
        "onepkg" | "onetoc2" => {
            let parser = Parser::new();
            let s = path.to_string_lossy();
            let notebook: Notebook = if kind == "onepkg" {
                parser.parse_package(make_typed(s.as_ref()))?
            } else {
                parser.parse_notebook(make_typed(s.as_ref()))?
            };
            notebook_pages(&notebook, &mut pages);
            if pages.is_empty() {
                anyhow::bail!(
                    "the notebook parsed to zero pages (sections could not be resolved); \
                     try pointing the tool at the notebook directory instead"
                );
            }
        }
        "one" => {
            pages.extend(section_pages(path)?);
        }
        _ => unreachable!(),
    }

    Ok(pages)
}

fn make_typed<'a>(s: &'a str) -> TypedPath<'a> {
    if std::path::MAIN_SEPARATOR == '\\' {
        TypedPath::windows(s)
    } else {
        TypedPath::unix(s)
    }
}

fn parse_section_with(path: &Path) -> anyhow::Result<Section> {
    let parser = Parser::new();
    let s = path.to_string_lossy();
    parser.parse_section(make_typed(s.as_ref())).map_err(Into::into)
}

fn notebook_pages(notebook: &Notebook, out: &mut Vec<PageData>) {
    for entry in notebook.entries() {
        section_entry_pages(entry, out);
    }
}

fn section_entry_pages(entry: &SectionEntry, out: &mut Vec<PageData>) {
    match entry {
        SectionEntry::Section(section) => {
            for series in section.page_series() {
                for page in series.pages() {
                    page_pages(page, out);
                }
            }
        }
        SectionEntry::SectionGroup(group) => {
            for entry in group.entries() {
                section_entry_pages(entry, out);
            }
        }
    }
}

fn section_pages(path: &Path) -> anyhow::Result<Vec<PageData>> {
    let section = parse_section_with(path)?;
    let mut out = Vec::new();
    for series in section.page_series() {
        for page in series.pages() {
            page_pages(page, &mut out);
        }
    }
    Ok(out)
}

fn page_pages(page: &Page, out: &mut Vec<PageData>) {
    let mut strokes = Vec::new();
    for content in page.contents() {
        if let PageContent::Ink(ink) = content {
            visit_ink(ink, (0.0, 0.0), &mut strokes);
        }
    }
    let height_half_inch = page.height().map(|h| h as f64);
    let title = page.title_text().map(str::to_string);
    out.push(PageData {
        strokes,
        height_half_inch,
        title,
    });
}

fn visit_ink(ink: &Ink, acc: (f64, f64), out: &mut Vec<InkStrokeData>) {
    let off = (
        acc.0 + ink.offset_horizontal().unwrap_or(0.0) as f64,
        acc.1 + ink.offset_vertical().unwrap_or(0.0) as f64,
    );

    for child in ink.child_groups() {
        visit_ink(child, off, out);
    }

    for stroke in ink.ink_strokes() {
        if let Some(data) = stroke_data(stroke, off) {
            out.push(data);
        }
    }
}

fn stroke_data(stroke: &InkStroke, off: (f64, f64)) -> Option<InkStrokeData> {
    let path = stroke.path();
    if path.len() < 2 {
        return None;
    }
    if stroke.width() as f64 <= 0.0 {
        return None;
    }

    let mut points = Vec::with_capacity(path.len());
    let (mut cx, mut cy) = (path[0].x() as f64, path[0].y() as f64);
    points.push((cx, cy));
    for p in &path[1..] {
        cx += p.x() as f64;
        cy += p.y() as f64;
        if let Some((lx, ly)) = points.last() {
            if (cx - *lx).abs() < 1e-6 && (cy - *ly).abs() < 1e-6 {
                continue;
            }
        }
        points.push((cx, cy));
    }
    if points.len() < 2 {
        return None;
    }

    Some(InkStrokeData {
        points,
        width_ink: stroke.width() as f64,
        color: stroke.color(),
        transparency: stroke.transparency(),
        off_half_inch: off,
    })
}

