use crate::onedata::{InkStrokeData, MediaData, MediaKind, PageData};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{Value, json};
use std::io::Write;

pub const INK_UNITS_PER_INCH: f64 = 2540.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    A4,
    UsLetter,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundKind {
    None,
    Lines,
    Grid,
}

pub struct Options {
    pub rnote_version: String,
    pub dpi: f64,
    pub format: FormatKind,
    pub min_page_height_mm: Option<f64>,
    pub margin_px: f64,
    pub background: BackgroundKind,
    pub normalize: bool,
    /// Place content exactly at its original OneNote coordinates (no margins / re-alignment).
    /// Mutually exclusive with `normalize`; when `true`, `normalize` is ignored.
    pub original_pos: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rnote_version: "0.15.0".to_string(),
            dpi: 96.0,
            format: FormatKind::Source,
            min_page_height_mm: None,
            margin_px: 48.0,
            background: BackgroundKind::Grid,
            normalize: true,
            original_pos: false,
        }
    }
}

/// A stroke prepared for output: points already in px, absolute within the page.
#[derive(Debug, Clone)]
pub struct OutStroke {
    pub points: Vec<(f64, f64)>,
    pub width_px: f64,
    pub color_rgba: (f64, f64, f64, f64),
    pub highlighter: bool,
}

/// A bitmap (image or rendered PDF page) prepared for output.
#[derive(Debug, Clone)]
pub struct OutImage {
    /// Top-left position in px, page-local.
    pub x: f64,
    /// Top-left position in px, page-local.
    pub y: f64,
    /// Display width in px.
    pub width: f64,
    /// Display height in px.
    pub height: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Raw RGBA8-premultiplied pixel data (`4 * pixel_width * pixel_height` bytes).
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PreparedPage {
    pub strokes: Vec<OutStroke>,
    pub images: Vec<OutImage>,
    pub height_half_inch: Option<f64>,
    pub guid: Option<String>,
    pub title: Option<String>,
}

impl PreparedPage {
    pub fn has_content(&self) -> bool {
        !self.strokes.is_empty() || !self.images.is_empty()
    }

    pub fn has_ink(&self) -> bool {
        !self.strokes.is_empty()
    }
}

fn dp3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn color_json(c: (f64, f64, f64, f64)) -> Value {
    let (r, g, b, a) = c;
    json!({"r": dp3(r), "g": dp3(g), "b": dp3(b), "a": dp3(a)})
}

fn stroke_json(stroke: &OutStroke) -> Value {
    let start = &stroke.points[0];
    let segments: Vec<Value> = stroke.points[1..]
        .iter()
        .map(|p| {
            json!({"lineto": {"end": {
                "pos": [dp3(p.0), dp3(p.1)],
                "pressure": 1.0
            }}})
        })
        .collect();

    json!({
        "brushstroke": {
            "path": {
                "start": {"pos": [dp3(start.0), dp3(start.1)], "pressure": 1.0},
                "segments": segments
            },
            "style": {
                "smooth": {
                    "stroke_width": dp3(stroke.width_px),
                    "stroke_color": color_json(stroke.color_rgba),
                    "fill_color": null,
                    "pressure_curve": "const",
                    "line_style": "solid",
                    "line_cap": "rounded"
                }
            }
        }
    })
}

/// A `Rectangle` (parry Cuboid + DAffine2) placed at `(x, y)` with the given extents.
///
/// The affine is the row-major 2D matrix `[x_axis, y_axis, translation]`, serialised by Rnote's
/// `DAffine2` as the 9-element array `[m11, m12, 0, m21, m22, 0, tx, ty, 1]` with the translation
/// at indices 6 and 7 (the center of the cuboid). For the 0.14 file format Rnote nests this under
/// a `transform` key; 0.15 uses a flat `affine` key.
fn rectangle_json(half_w: f64, half_h: f64, cx: f64, cy: f64, v014: bool) -> Value {
    let affine = json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, dp3(cx), dp3(cy), 1.0]);
    let cuboid = json!({"half_extents": [dp3(half_w), dp3(half_h)]});
    if v014 {
        json!({"cuboid": cuboid, "transform": {"affine": affine}})
    } else {
        json!({"cuboid": cuboid, "affine": affine})
    }
}

fn image_json(image: &OutImage, v014: bool) -> Value {
    // Display rectangle spans `x..x+width` x `y..y+height` (center = x+w/2, y+h/2).
    let display = rectangle_json(
        image.width * 0.5,
        image.height * 0.5,
        image.x + image.width * 0.5,
        image.y + image.height * 0.5,
        v014,
    );
    // The embedded image's own rectangle spans 0..pixel_width x 0..pixel_height.
    let image_rect = rectangle_json(
        image.pixel_width as f64 * 0.5,
        image.pixel_height as f64 * 0.5,
        image.pixel_width as f64 * 0.5,
        image.pixel_height as f64 * 0.5,
        v014,
    );

    json!({
        "bitmapimage": {
            "image": {
                "data": base64_encode(&image.data),
                "rectangle": image_rect,
                "pixel_width": image.pixel_width,
                "pixel_height": image.pixel_height,
                "memory_format": "R8g8b8a8Premultiplied"
            },
            "rectangle": display
        }
    })
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode an encoded image (PNG/JPEG) into raw RGBA8-premultiplied pixels.
fn decode_to_premul_rgba(encoded: &[u8]) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(encoded)
        .map_err(|e| anyhow::anyhow!("decoding image failed: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for px in rgba.pixels() {
        let (r, g, b, a) = (px.0[0], px.0[1], px.0[2], px.0[3]);
        // Premultiply: Rnote stores R8g8b8a8Premultiplied.
        data.extend_from_slice(&[
            (r as u16 * a as u16 / 255) as u8,
            (g as u16 * a as u16 / 255) as u8,
            (b as u16 * a as u16 / 255) as u8,
            a,
        ]);
    }
    Ok((data, w, h))
}

/// Convert a media item into an output image. `Pdf` items are rendered via `pdftoppm`.
fn convert_media(media: &MediaData, dpi: f64) -> anyhow::Result<Option<OutImage>> {
    let scale = dpi / 2.0; // half-inch -> px
    let x = media.x_half_inch * scale;
    let y = media.y_half_inch * scale;

    let (data, pixel_width, pixel_height) = match media.kind {
        MediaKind::Image => decode_to_premul_rgba(&media.bytes)?,
        MediaKind::Pdf => {
            let page_index = media.page_index.unwrap_or(0);
            let png = crate::pdf::render_pdf_page(&media.bytes, page_index, dpi as u32)?;
            decode_to_premul_rgba(&png)?
        }
        MediaKind::Other => return Ok(None),
    };

    let mut width = media.width_half_inch * scale;
    let mut height = media.height_half_inch * scale;
    if width <= 0.0 || height <= 0.0 {
        width = pixel_width as f64;
        height = pixel_height as f64;
    }

    Ok(Some(OutImage {
        x,
        y,
        width,
        height,
        pixel_width,
        pixel_height,
        data,
    }))
}

pub fn build_rnote_bytes(pages: &[PreparedPage], options: &Options) -> anyhow::Result<Vec<u8>> {
    let nonempty: Vec<&PreparedPage> = pages.iter().filter(|p| p.has_content()).collect();

    if nonempty.is_empty() {
        anyhow::bail!("no handwritten ink or media found in the input files");
    }

    let (page_w_px, page_h_px) = compute_page_size(&nonempty, options);

    // Re-position content onto the page grid.
    let offsets: Vec<(f64, f64)> = if options.original_pos {
        nonempty.iter().map(|_| (0.0, 0.0)).collect()
    } else {
        nonempty
            .iter()
            .enumerate()
            .map(|(idx, page)| page_offset(idx, page, page_h_px, options))
            .collect()
    };

    build_document(&nonempty, &offsets, page_w_px, page_h_px, options)
}

/// Build a single-page `.rnote` document for one page (used by `--out-dir`).
/// Content is kept at its original position.
pub fn build_rnote_bytes_single(page: &PreparedPage, options: &Options) -> anyhow::Result<Vec<u8>> {
    let (page_w_px, page_h_px) = compute_single_page_size(page, options);
    let pages = std::slice::from_ref(&page);
    let offsets = vec![(0.0, 0.0)];
    build_document(pages, &offsets, page_w_px, page_h_px, options)
}

/// Page geometry in millimetres, from the configured format.
fn page_size_mm(options: &Options) -> (f64, f64) {
    match options.format {
        FormatKind::A4 => (210.0, 297.0),
        FormatKind::UsLetter => (215.9, 279.4),
        FormatKind::Source => (210.0, 297.0),
    }
}

/// The page is always the configured size (A4 by default). Content is not fitted or stretched.
fn compute_page_size(nonempty: &[&PreparedPage], options: &Options) -> (f64, f64) {
    let (page_w_mm, mut page_h_mm) = page_size_mm(options);

    if options.format == FormatKind::Source {
        // Honour the OneNote page height when available.
        for page in nonempty.iter() {
            if let Some(half_inch) = page.height_half_inch {
                let h_mm = half_inch * 0.5 * 25.4;
                if (60.0..=1500.0).contains(&h_mm) {
                    page_h_mm = page_h_mm.max(h_mm);
                }
            }
        }
    }
    if let Some(min_mm) = options.min_page_height_mm {
        page_h_mm = page_h_mm.max(min_mm);
    }

    let mm_to_px = |mm: f64| mm / 25.4 * options.dpi;
    let page_w_px = mm_to_px(page_w_mm).max(60.0);
    let page_h_px = mm_to_px(page_h_mm).max(60.0);
    (page_w_px, page_h_px)
}

fn compute_single_page_size(page: &PreparedPage, options: &Options) -> (f64, f64) {
    let one = [page];
    compute_page_size(&one, options)
}

fn page_offset(
    page_idx: usize,
    page: &PreparedPage,
    page_h_px: f64,
    options: &Options,
) -> (f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    for s in page.strokes.iter() {
        let hw = s.width_px * 0.5;
        for (x, y) in s.points.iter() {
            min_x = min_x.min(x - hw);
            min_y = min_y.min(y - hw);
        }
    }
    for img in page.images.iter() {
        min_x = min_x.min(img.x);
        min_y = min_y.min(img.y);
    }

    if !min_x.is_finite() {
        (0.0, page_idx as f64 * page_h_px)
    } else if options.normalize {
        (
            options.margin_px - min_x,
            page_idx as f64 * page_h_px + options.margin_px - min_y,
        )
    } else {
        (0.0, page_idx as f64 * page_h_px)
    }
}

fn build_document(
    pages: &[&PreparedPage],
    offsets: &[(f64, f64)],
    page_w_px: f64,
    page_h_px: f64,
    options: &Options,
) -> anyhow::Result<Vec<u8>> {
    let n_pages = pages.len();
    let doc_height = dp3(page_h_px * n_pages as f64);
    let doc_width = dp3(page_w_px);
    let mm_to_px = |mm: f64| mm / 25.4 * options.dpi;

    let (bord_r, bord_g, bord_b) = (0.298, 0.318, 0.341);
    let background = match options.background {
        BackgroundKind::None => json!({
            "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0},
            "pattern": "none",
            "pattern_size": [mm_to_px(5.0), mm_to_px(5.0)],
            "pattern_color": {"r": 0.8, "g": 0.9, "b": 1.0, "a": 1.0}
        }),
        BackgroundKind::Lines => json!({
            "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0},
            "pattern": "lines",
            "pattern_size": [dp3(mm_to_px(3.0)), dp3(mm_to_px(8.0))],
            "pattern_color": {"r": 0.8, "g": 0.8, "b": 0.8, "a": 1.0}
        }),
        BackgroundKind::Grid => json!({
            "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0},
            "pattern": "grid",
            "pattern_size": [22.0, 22.0],
            "pattern_color": {"r": 0.8, "g": 0.8, "b": 0.8, "a": 1.0}
        }),
    };

    let orientation = if page_w_px <= page_h_px {
        "portrait"
    } else {
        "landscape"
    };

    // Single-page documents use the infinite canvas; multi-page single files stack vertically.
    let layout = if n_pages == 1 { "infinite" } else { "continuous_vertical" };

    let document = json!({
        "config": {
            "format": {
                "width": dp3(page_w_px),
                "height": dp3(page_h_px),
                "dpi": dp3(options.dpi),
                "orientation": orientation,
                "border_color": {"r": bord_r, "g": bord_g, "b": bord_b, "a": 1.0},
                "show_borders": false,
                "show_origin_indicator": false
            },
            "background": background,
            "layout": layout
        },
        "x": 0.0,
        "y": 0.0,
        "width": doc_width,
        "height": doc_height
    });

    let camera = json!({
        "offset": [-96.0, -96.0],
        "size": [800.0, 600.0],
        "zoom": 1.0
    });

    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(5));
    write!(
        encoder,
        r#"{{"version":"{}","data":{{"engine_snapshot":{{"document":{},"camera":{},"stroke_components":["#,
        options.rnote_version,
        serde_json::to_string(&document)?,
        serde_json::to_string(&camera)?,
    )?;
    write!(encoder, r#"{{"value":null,"version":0}}"#)?;

    // The 0.14 file format nests the rectangle affine under a `transform` key; 0.15 uses `affine`.
    let v014 = options.rnote_version.starts_with("0.14");

    // Flatten all components in page order (strokes then images per page).
    let mut components: Vec<Value> = Vec::new();
    for (page, offset) in pages.iter().zip(offsets.iter()) {
        let (dx, dy) = *offset;
        for s in page.strokes.iter() {
            let moved = OutStroke {
                points: s.points.iter().map(|(x, y)| (x + dx, y + dy)).collect(),
                width_px: s.width_px,
                color_rgba: s.color_rgba,
                highlighter: s.highlighter,
            };
            components.push(stroke_json(&moved));
        }
        for img in page.images.iter() {
            let moved = OutImage {
                x: img.x + dx,
                y: img.y + dy,
                ..img.clone()
            };
            components.push(image_json(&moved, v014));
        }
    }

    for comp in &components {
        write!(
            encoder,
            ",{}",
            serde_json::to_string(&json!({"value": comp, "version": 1}))?
        )?;
    }

    write!(encoder, r#"],"chrono_components":[{{"value":null,"version":0}}"#)?;
    let mut t: u32 = 0;
    for (page, _offset) in pages.iter().zip(offsets.iter()) {
        for s in page.strokes.iter() {
            t += 1;
            let layer = if s.highlighter {
                json!("highlighter")
            } else {
                json!({"user_layer": 0})
            };
            write!(
                encoder,
                ",{}",
                serde_json::to_string(&json!({"value": {"t": t, "layer": layer}, "version": 1}))?
            )?;
        }
        for _img in page.images.iter() {
            t += 1;
            write!(
                encoder,
                ",{}",
                serde_json::to_string(&json!({"value": {"t": t, "layer": "image"}, "version": 1}))?
            )?;
        }
    }

    write!(encoder, r#"],"chrono_counter":{}"#, t)?;
    encoder.write_all(b"}}}")?;

    Ok(encoder.finish()?)
}

/// Convert one OneNote page into output content (strokes + images) in pixels.
/// May render PDF media, so it can be somewhat expensive — call it only for pages you export.
pub fn prepare_page(page: &PageData, options: &Options) -> anyhow::Result<PreparedPage> {
    let strokes: Vec<OutStroke> = page
        .strokes
        .iter()
        .map(|s| convert_stroke(s, options.dpi))
        .collect();

    // A multi-page printout is represented by one preview image per page; the original PDF is
    // only exported as a sidecar and must not be embedded again on top of its previews.
    let has_previews = page.media.iter().any(|m| m.is_preview);

    const MIN_POS: f64 = 0.1; // half-inch
    // Rnote's own PDF import places successive pages `height + IMPORT_OFFSET_DEFAULT[1]*0.5`
    // apart, with `IMPORT_OFFSET_DEFAULT = 32` -> a 16 px vertical gap between pages.
    const FLOW_GAP_PX: f64 = 16.0;
    const OVERLAP_TOL_PX: f64 = 2.0;

    let mut images: Vec<OutImage> = Vec::new();

    // Printout pages are laid out exactly like Rnote's PDF import: every page below the previous
    // one, starting at the top, so the pages never stack on top of each other.
    if has_previews {
        let mut y = 0.0f64;
        for media in &page.media {
            if !media.is_preview {
                continue;
            }
            if let Some(mut img) = convert_media(media, options.dpi)? {
                img.x = 0.0;
                img.y = y;
                images.push(img);
                y += images.last().unwrap().height + FLOW_GAP_PX;
            }
        }
    }

    // Other images (photos, scans, standalone PDFs): keep a usable position, otherwise flow.
    // They are pushed down until they no longer overlap an already placed image, so they always
    // sit below the printout (or below each other) and never stack.
    let mut flow_y = images
        .iter()
        .map(|i| i.y + i.height + FLOW_GAP_PX)
        .fold(0.0f64, f64::max);
    for media in &page.media {
        if media.is_preview {
            continue;
        }
        if media.kind == MediaKind::Pdf && has_previews {
            continue; // printout original is only exported as the merged sidecar
        }
        let Some(mut img) = convert_media(media, options.dpi)? else {
            continue;
        };
        let positioned =
            media.x_half_inch.abs() > MIN_POS || media.y_half_inch.abs() > MIN_POS;
        if !positioned {
            img.x = 0.0;
            img.y = flow_y;
        }
        // Move the image down until it no longer overlaps any previously placed image.
        let mut guard = 0usize;
        loop {
            let mut overlapped = false;
            for other in &images {
                if rects_overlap(&img, other, OVERLAP_TOL_PX) {
                    img.y = other.y + other.height + FLOW_GAP_PX;
                    overlapped = true;
                }
            }
            guard += 1;
            if !overlapped || guard > images.len() + 1 {
                break;
            }
        }
        let bottom = img.y + img.height + FLOW_GAP_PX;
        images.push(img);
        flow_y = flow_y.max(bottom);
    }
    Ok(PreparedPage {
        strokes,
        images,
        height_half_inch: page.height_half_inch,
        guid: page.guid.clone(),
        title: page.title.clone(),
    })
}

/// True when two images overlap by more than `tol` pixels (both axes, so edge-touching is ignored).
fn rects_overlap(a: &OutImage, b: &OutImage, tol: f64) -> bool {
    a.x < b.x + b.width - tol
        && b.x < a.x + a.width - tol
        && a.y < b.y + b.height - tol
        && b.y < a.y + a.height - tol
}

/// Convert OneNote page data into output content (strokes + images) in pixels.
pub fn prepare_strokes(pages: &[PageData], options: &Options) -> anyhow::Result<Vec<PreparedPage>> {
    let mut out = Vec::with_capacity(pages.len());
    for page in pages {
        out.push(prepare_page(page, options)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    fn build_sample() -> PreparedPage {
        let strokes = vec![OutStroke {
            points: vec![(0.0, 0.0), (10.0, 10.0)],
            width_px: 2.0,
            color_rgba: (0.0, 0.0, 0.0, 1.0),
            highlighter: false,
        }];
        // 2x2 RGBA8-premultiplied image.
        let data = vec![255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
        let images = vec![OutImage {
            x: 5.0,
            y: 6.0,
            width: 100.0,
            height: 80.0,
            pixel_width: 2,
            pixel_height: 2,
            data,
        }];
        PreparedPage {
            strokes,
            images,
            height_half_inch: Some(600.0),
            guid: Some("guid".to_string()),
            title: Some("t".to_string()),
        }
    }

    fn gunzip(bytes: &[u8]) -> Value {
        let mut s = String::new();
        GzDecoder::new(&bytes[..]).read_to_string(&mut s).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn bitmapimage_schema_matches_rnote() {
        let page = build_sample();
        let options = Options {
            format: FormatKind::A4,
            original_pos: true,
            ..Default::default()
        };
        let bytes = build_rnote_bytes_single(&page, &options).unwrap();
        let root = gunzip(&bytes);
        let sc = &root["data"]["engine_snapshot"]["stroke_components"];
        let cc = &root["data"]["engine_snapshot"]["chrono_components"];
        assert_eq!(sc.as_array().unwrap().len(), cc.as_array().unwrap().len());

        // slot 0 = empty sentinel, slot 1 = stroke, slot 2 = image
        let image = &sc[2]["value"]["bitmapimage"];
        assert!(image["image"]["data"].is_string());
        assert_eq!(image["image"]["memory_format"], "R8g8b8a8Premultiplied");
        assert_eq!(image["image"]["pixel_width"], 2);
        assert_eq!(image["image"]["pixel_height"], 2);
        // image rectangle: identity transform centered at (1,1) -> affine [1,0,0, 0,1,0, 1,1,1]
        assert_eq!(
            image["image"]["rectangle"]["affine"],
            json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(
            image["image"]["rectangle"]["cuboid"]["half_extents"],
            json!([1.0, 1.0])
        );
        // display rectangle: center at (5+50, 6+40) = (55, 46)
        assert_eq!(
            image["rectangle"]["affine"],
            json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 55.0, 46.0, 1.0])
        );
        assert_eq!(
            image["rectangle"]["cuboid"]["half_extents"],
            json!([50.0, 40.0])
        );
        // stroke on user_layer 0 (slot 1), image on "image" layer (slot 2)
        assert_eq!(cc[1]["value"]["layer"], json!({"user_layer": 0}));
        assert_eq!(cc[2]["value"]["layer"], "image");
        // chrono_counter counts both components
        assert_eq!(root["data"]["engine_snapshot"]["chrono_counter"], 2);

        // Defaults: infinite canvas, grid background with 22px tiles, invisible borders.
        let doc = &root["data"]["engine_snapshot"]["document"];
        assert_eq!(doc["config"]["layout"], "infinite");
        assert_eq!(doc["config"]["format"]["show_borders"], false);
        assert_eq!(doc["config"]["format"]["show_origin_indicator"], false);
        assert_eq!(doc["config"]["background"]["pattern"], "grid");
        assert_eq!(doc["config"]["background"]["pattern_size"], json!([22.0, 22.0]));
    }

    #[test]
    fn rnote_014_uses_nested_transform_affine() {
        // Rnote 0.14 nests the rectangle affine under a `transform` key (9-element row-major).
        let page = build_sample();
        let options = Options {
            rnote_version: "0.14.2".to_string(),
            format: FormatKind::A4,
            original_pos: true,
            ..Default::default()
        };
        let bytes = build_rnote_bytes_single(&page, &options).unwrap();
        let root = gunzip(&bytes);
        let sc = &root["data"]["engine_snapshot"]["stroke_components"];
        assert_eq!(root["version"], "0.14.2");
        let image = &sc[2]["value"]["bitmapimage"];
        assert_eq!(
            image["rectangle"]["transform"]["affine"],
            json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 55.0, 46.0, 1.0])
        );
        assert!(image["rectangle"].get("affine").is_none(), "0.14 must not use a flat affine");
    }

    #[test]
    fn zero_offset_images_flow_vertically() {
        // Two images with no usable position must not overlap (laid out in a vertical flow).
        const ONE_PX_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(ONE_PX_PNG)
            .expect("valid png base64");
        let mk_media = |w: f64, h: f64| crate::onedata::MediaData {
            kind: crate::onedata::MediaKind::Image,
            x_half_inch: 0.0,
            y_half_inch: 0.0,
            width_half_inch: w,
            height_half_inch: h,
            filename: "img.png".to_string(),
            bytes: png.clone(),
            page_index: None,
            is_preview: false,
        };
        // widths/heights in half-inches: 2 half-in = 1 inch = 96px at 96 dpi.
        let page = crate::onedata::PageData {
            strokes: Vec::new(),
            media: vec![mk_media(2.0, 2.0), mk_media(2.0, 2.0)],
            guid: None,
            updated_time: 0,
            height_half_inch: None,
            title: None,
        };
        let options = Options::default(); // dpi 96
        let prepared = prepare_page(&page, &options).unwrap();
        let bytes = build_rnote_bytes_single(&prepared, &options).unwrap();
        let root = gunzip(&bytes);
        let sc = &root["data"]["engine_snapshot"]["stroke_components"];
        // slot 1 = image 1 (at 0,0), slot 2 = image 2 (flowed below image 1)
        let img1 = &sc[1]["value"]["bitmapimage"];
        let img2 = &sc[2]["value"]["bitmapimage"];
        let h = img1["rectangle"]["cuboid"]["half_extents"][1].as_f64().unwrap();
        let ty2 = img2["rectangle"]["affine"][7].as_f64().unwrap();
        let h2 = img2["rectangle"]["cuboid"]["half_extents"][1].as_f64().unwrap();
        let top2 = ty2 - h2;
        // image1 spans 0..(2*h); image2 top = 2*h + 16 gap
        let expected = 2.0 * h + 16.0;
        assert!((top2 - expected).abs() < 1.0, "image 2 should start at {expected}, got {top2}");
    }

    #[test]
    fn same_position_images_are_separated_vertically() {
        // Two images placed at the exact same OneNote position (e.g. an embedded PDF and a pasted
        // photo of the same sheet) must not overlap: the second one is pushed below the first.
        const ONE_PX_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(ONE_PX_PNG)
            .expect("valid png base64");
        let mk_media = |w: f64, h: f64| crate::onedata::MediaData {
            kind: crate::onedata::MediaKind::Image,
            x_half_inch: 1.0,
            y_half_inch: 2.4,
            width_half_inch: w,
            height_half_inch: h,
            filename: "media.png".to_string(),
            bytes: png.clone(),
            page_index: None,
            is_preview: false,
        };
        let page = crate::onedata::PageData {
            strokes: Vec::new(),
            media: vec![mk_media(2.0, 2.0), mk_media(2.0, 2.0)],
            guid: None,
            updated_time: 0,
            height_half_inch: None,
            title: None,
        };
        let options = Options::default(); // dpi 96
        let prepared = prepare_page(&page, &options).unwrap();
        assert_eq!(prepared.images.len(), 2);
        let (a, b) = (&prepared.images[0], &prepared.images[1]);
        assert!((a.x - 48.0).abs() < 1.0 && (a.y - 115.2).abs() < 1.0, "first keeps its position");
        // Both start with the same origin; the second must have been pushed down out of the first.
        assert!(b.y >= a.y + a.height + 16.0 - 0.01, "second must sit below the first");
    }

    #[test]
    fn printout_previews_are_kept_and_original_pdf_not_embedded() {
        // A PDF printout page: N preview images (displayed_page_number set) + the original PDF.
        // prepare_page must embed the previews but NOT render the original PDF again.
        const ONE_PX_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(ONE_PX_PNG)
            .expect("valid png base64");
        let mk_preview = |y: f64| crate::onedata::MediaData {
            kind: crate::onedata::MediaKind::Image,
            x_half_inch: 1.0,
            y_half_inch: y,
            width_half_inch: 2.0,
            height_half_inch: 2.0,
            filename: "printout_2.pdf".to_string(),
            bytes: png.clone(),
            page_index: Some(2),
            is_preview: true,
        };
        let original = crate::onedata::MediaData {
            kind: crate::onedata::MediaKind::Pdf,
            x_half_inch: 1.0,
            y_half_inch: 2.4,
            width_half_inch: 2.0,
            height_half_inch: 2.0,
            filename: "printout.pdf".to_string(),
            bytes: png.clone(),
            page_index: None,
            is_preview: false,
        };
        let page = crate::onedata::PageData {
            strokes: Vec::new(),
            media: vec![mk_preview(2.4), mk_preview(24.0), original],
            guid: None,
            updated_time: 0,
            height_half_inch: None,
            title: None,
        };
        let options = Options::default();
        let prepared = prepare_page(&page, &options).unwrap();
        // Only the two previews are embedded; the original PDF is not re-rendered.
        assert_eq!(prepared.images.len(), 2);
        for img in &prepared.images {
            assert_eq!(img.pixel_width, 1, "preview decoded as a bitmap, not rendered as PDF");
        }
    }

    #[test]
    fn image_data_is_base64_premul_pixels() {
        let page = build_sample();
        let options = Options {
            format: FormatKind::A4,
            original_pos: true,
            ..Default::default()
        };
        let bytes = build_rnote_bytes_single(&page, &options).unwrap();
        let root = gunzip(&bytes);
        let data = root["data"]["engine_snapshot"]["stroke_components"][2]["value"]
            ["bitmapimage"]["image"]["data"]
            .as_str()
            .unwrap()
            .to_string();
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .unwrap();
        assert_eq!(decoded.len(), 4 * 2 * 2);
    }
}

fn convert_stroke(s: &InkStrokeData, dpi: f64) -> OutStroke {
    let scale = dpi / INK_UNITS_PER_INCH;
    let off_scale = dpi / 2.0;
    let ox = s.off_half_inch.0 * off_scale;
    let oy = s.off_half_inch.1 * off_scale;

    let points: Vec<(f64, f64)> = s
        .points
        .iter()
        .map(|(x, y)| (x * scale + ox, y * scale + oy))
        .collect();

    let width_px = (s.width_ink * scale).clamp(0.5, 60.0);

    let (r, g, b) = match s.color {
        Some(value) => {
            let r = (value & 0xFF) as f64 / 255.0;
            let g = ((value >> 8) & 0xFF) as f64 / 255.0;
            let b = ((value >> 16) & 0xFF) as f64 / 255.0;
            (r, g, b)
        }
        None => (0.0, 0.0, 0.0),
    };
    let a = (255.0 - s.transparency.unwrap_or(0) as f64) / 255.0;

    let highlighter = a < 0.55;

    OutStroke {
        points,
        width_px,
        color_rgba: (r, g, b, a.clamp(0.0, 1.0)),
        highlighter,
    }
}
