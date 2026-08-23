use onenote_parser::contents::{
    Content, EmbeddedFile, FileDataStatus, Ink, InkStroke, OutlineElement, OutlineItem,
};
use onenote_parser::notebook::Notebook;
use onenote_parser::page::{Page, PageContent};
use onenote_parser::section::{Section, SectionEntry};
use onenote_parser::Parser;
use sanitize_filename::sanitize;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::Path;
use typed_path::TypedPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A raster image to be embedded (and provided as a sidecar file).
    Image,
    /// A PDF (or XPS) to be rendered to a bitmap and embedded; the original is provided as a sidecar.
    Pdf,
    /// Any other embedded file; only provided as a sidecar, not embedded.
    Other,
}

#[derive(Debug, Clone)]
pub struct InkStrokeData {
    pub points: Vec<(f64, f64)>,
    pub width_ink: f64,
    pub color: Option<u32>,
    pub transparency: Option<u8>,
    pub off_half_inch: (f64, f64),
}

/// A media item on a page (image or embedded file).
#[derive(Debug, Clone)]
pub struct MediaData {
    pub kind: MediaKind,
    /// Horizontal offset from the page origin, in half-inch increments.
    pub x_half_inch: f64,
    /// Vertical offset from the page origin, in half-inch increments.
    pub y_half_inch: f64,
    /// Display width, in half-inch increments.
    pub width_half_inch: f64,
    /// Display height, in half-inch increments.
    pub height_half_inch: f64,
    /// Original file name, sanitised for safe use as a path component.
    pub filename: String,
    /// The binary payload.
    pub bytes: Vec<u8>,
    /// Page number to display for multi-page files (0-based), if known.
    pub page_index: Option<u32>,
    /// True for the per-page preview images of an inserted multi-page printout (PDF/XPS).
    /// These are only visual placeholders for the original file, which is exported separately.
    pub is_preview: bool,
}

#[derive(Debug, Clone)]
pub struct PageData {
    pub strokes: Vec<InkStrokeData>,
    pub media: Vec<MediaData>,
    /// The page GUID used as a stable identity for incremental updates.
    pub guid: Option<String>,
    /// Unix timestamp of the last modification.
    pub updated_time: i64,
    pub height_half_inch: Option<f64>,
    pub title: Option<String>,
}

impl PageData {
    pub fn has_ink(&self) -> bool {
        !self.strokes.is_empty()
    }

    pub fn has_any_content(&self) -> bool {
        self.has_ink() || !self.media.is_empty()
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
    let mut media = Vec::new();
    for content in page.contents() {
        visit_page_content(content, (0.0, 0.0), &mut strokes, &mut media);
    }
    let height_half_inch = page.height().map(|h| h as f64);
    let title = page.title_text().map(str::to_string);
    let guid = {
        let id = page.link_target_id();
        if id.is_empty() {
            None
        } else {
            Some(id.to_string())
        }
    };
    let updated_time = page.updated_time().unix_timestamp();
    out.push(PageData {
        strokes,
        media,
        guid,
        updated_time,
        height_half_inch,
        title,
    });
}

fn visit_page_content(
    content: &PageContent,
    acc: (f64, f64),
    strokes: &mut Vec<InkStrokeData>,
    media: &mut Vec<MediaData>,
) {
    match content {
        PageContent::Ink(ink) => visit_ink(ink, acc, strokes),
        PageContent::Image(image) => {
            if let Some(m) = image_media(image, acc) {
                media.push(m);
            }
        }
        PageContent::EmbeddedFile(embedded) => {
            if let Some(m) = embedded_media(embedded, acc) {
                media.push(m);
            }
        }
        PageContent::Outline(outline) => {
            let off = (
                acc.0 + outline.offset_horizontal().unwrap_or(0.0) as f64,
                acc.1 + outline.offset_vertical().unwrap_or(0.0) as f64,
            );
            for item in outline.items() {
                visit_outline_item(item, off, strokes, media);
            }
        }
        _ => {}
    }
}

fn visit_outline_item(
    item: &OutlineItem,
    acc: (f64, f64),
    strokes: &mut Vec<InkStrokeData>,
    media: &mut Vec<MediaData>,
) {
    match item {
        OutlineItem::Element(element) => visit_outline_element(element, acc, strokes, media),
        OutlineItem::Group(group) => {
            for item in group.outlines() {
                visit_outline_item(item, acc, strokes, media);
            }
        }
    }
}

fn visit_outline_element(
    element: &OutlineElement,
    acc: (f64, f64),
    strokes: &mut Vec<InkStrokeData>,
    media: &mut Vec<MediaData>,
) {
    for content in element.contents() {
        match content {
            Content::Ink(ink) => visit_ink(ink, acc, strokes),
            Content::Image(image) => {
                if let Some(m) = image_media(image, acc) {
                    media.push(m);
                }
            }
            Content::EmbeddedFile(embedded) => {
                if let Some(m) = embedded_media(embedded, acc) {
                    media.push(m);
                }
            }
            _ => {}
        }
    }
    for item in element.children() {
        visit_outline_item(item, acc, strokes, media);
    }
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

fn image_media(
    image: &onenote_parser::contents::Image,
    acc: (f64, f64),
) -> Option<MediaData> {
    if image.data_status() != FileDataStatus::Available {
        return None;
    }
    let mut reader = image.read()?;
    let mut bytes = Vec::new();
    if reader.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
        return None;
    }

    let x = acc.0 + image.offset_horizontal().unwrap_or(0.0) as f64;
    let y = acc.1 + image.offset_vertical().unwrap_or(0.0) as f64;
    let width = image
        .picture_width()
        .or_else(|| image.layout_max_width())
        .unwrap_or(0.0) as f64;
    let height = image
        .picture_height()
        .or_else(|| image.layout_max_height())
        .unwrap_or(0.0) as f64;

    let raw_name = image
        .image_filename()
        .or_else(|| image.alt_text())
        .map(str::to_string)
        .unwrap_or_else(|| "image".to_string());
    let ext = image.extension().map(str::to_lowercase);
    let is_multi_page_file = ext.as_deref().is_some_and(|e| matches!(e, "pdf" | "xps"));

    let kind = if is_multi_page_file {
        MediaKind::Pdf
    } else {
        MediaKind::Image
    };

    // OneNote stores an inserted PDF/XPS printout as one preview image per page; each such
    // image carries a displayed page number and is only a visual placeholder for the original
    // multi-page file (which is exported separately as a single PDF sidecar).
    let is_preview = image.displayed_page_number().is_some();

    Some(MediaData {
        kind,
        x_half_inch: x,
        y_half_inch: y,
        width_half_inch: width,
        height_half_inch: height,
        filename: sanitize(&raw_name),
        bytes,
        page_index: image.displayed_page_number(),
        is_preview,
    })
}

fn embedded_media(embedded: &EmbeddedFile, acc: (f64, f64)) -> Option<MediaData> {
    if embedded.data_status() != FileDataStatus::Available {
        return None;
    }
    let mut reader = embedded.read();
    let mut bytes = Vec::new();
    if reader.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
        return None;
    }

    let raw_name = embedded.filename();
    let ext = Path::new(raw_name)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_lowercase);
    let kind = match ext.as_deref() {
        Some("pdf") | Some("xps") => MediaKind::Pdf,
        _ => MediaKind::Other,
    };

    let x = acc.0 + embedded.offset_horizontal().unwrap_or(0.0) as f64;
    let y = acc.1 + embedded.offset_vertical().unwrap_or(0.0) as f64;
    let width = embedded.layout_max_width().unwrap_or(0.0) as f64;
    let height = embedded.layout_max_height().unwrap_or(0.0) as f64;

    Some(MediaData {
        kind,
        x_half_inch: x,
        y_half_inch: y,
        width_half_inch: width,
        height_half_inch: height,
        filename: sanitize(raw_name),
        bytes,
        page_index: None,
        is_preview: false,
    })
}
