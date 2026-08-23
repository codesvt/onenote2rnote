use crate::onedata::InkStrokeData;
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
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rnote_version: "0.15.0".to_string(),
            dpi: 96.0,
            format: FormatKind::Source,
            min_page_height_mm: None,
            margin_px: 48.0,
            background: BackgroundKind::Lines,
            normalize: true,
        }
    }
}

/// A stroke prepared for output: points already in px, absolute within the doc.
#[derive(Debug, Clone)]
pub struct OutStroke {
    pub points: Vec<(f64, f64)>,
    pub width_px: f64,
    pub color_rgba: (f64, f64, f64, f64),
    pub highlighter: bool,
}

#[derive(Debug, Clone)]
pub struct PreparedPage {
    pub strokes: Vec<OutStroke>,
    pub height_half_inch: Option<f64>,
}

impl PreparedPage {
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

pub fn build_rnote_bytes(pages: &[PreparedPage], options: &Options) -> anyhow::Result<Vec<u8>> {
    let nonempty: Vec<&PreparedPage> = pages.iter().filter(|p| p.has_ink()).collect();

    if nonempty.is_empty() {
        anyhow::bail!("no handwritten ink found in the input files");
    }

    let mm_to_px = |mm: f64| mm / 25.4 * options.dpi;

    // Geometry per page: bounding boxes (in px, already page-local).
    let mut content_w: f64 = 0.0;
    let mut content_h: f64 = 0.0;
    for page in &nonempty {
        let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
        let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for s in page.strokes.iter() {
            let hw = s.width_px * 0.5;
            for (x, y) in s.points.iter() {
                min_x = min_x.min(x - hw);
                min_y = min_y.min(y - hw);
                max_x = max_x.max(x + hw);
                max_y = max_y.max(y + hw);
            }
        }
        if max_x > min_x && max_y > min_y {
            content_w = content_w.max(max_x - min_x);
            content_h = content_h.max(max_y - min_y);
        }
    }

    let (page_w_mm, mut page_h_mm): (f64, f64) = match options.format {
        FormatKind::A4 => (210.0, 297.0),
        FormatKind::UsLetter => (215.9, 279.4),
        FormatKind::Source => (210.0, 297.0),
    };

    if options.format == FormatKind::Source {
        // honour the OneNote page height when available
        for page in pages.iter() {
            if let Some(half_inch) = page.height_half_inch {
                let h_mm = half_inch * 0.5 * 25.4;
                if h_mm >= 60.0 && h_mm <= 1500.0 {
                    page_h_mm = page_h_mm.max(h_mm);
                }
            }
        }
    }
    if let Some(min_mm) = options.min_page_height_mm {
        page_h_mm = page_h_mm.max(min_mm);
    }

    let mut page_w_px = mm_to_px(page_w_mm).max(content_w + 2.0 * options.margin_px);
    let mut page_h_px = mm_to_px(page_h_mm).max(content_h + 2.0 * options.margin_px);

    if page_h_px < 60.0 {
        page_h_px = 60.0;
    }
    if page_w_px < 60.0 {
        page_w_px = 60.0;
    }

    // Re-position strokes onto the final page grid.
    let n_pages = nonempty.len();
    let doc_height = dp3(page_h_px * n_pages as f64);
    let doc_width = dp3(page_w_px);

    let page_offset = |page_idx: usize, page: &PreparedPage| -> (f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        for s in page.strokes.iter() {
            let hw = s.width_px * 0.5;
            for (x, y) in s.points.iter() {
                min_x = min_x.min(x - hw);
                min_y = min_y.min(y - hw);
            }
        }
        if !min_x.is_finite() {
            (0.0, 0.0)
        } else if options.normalize {
            (
                options.margin_px - min_x,
                page_idx as f64 * page_h_px + options.margin_px - min_y,
            )
        } else {
            (0.0, page_idx as f64 * page_h_px)
        }
    };

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
            "pattern_size": [dp3(mm_to_px(5.0)), dp3(mm_to_px(5.0))],
            "pattern_color": {"r": 0.8, "g": 0.8, "b": 0.8, "a": 1.0}
        }),
    };

    let orientation = if page_w_px <= page_h_px { "portrait" } else { "landscape" };

    let document = json!({
        "config": {
            "format": {
                "width": dp3(page_w_px),
                "height": dp3(page_h_px),
                "dpi": dp3(options.dpi),
                "orientation": orientation,
                "border_color": {"r": bord_r, "g": bord_g, "b": bord_b, "a": 1.0},
                "show_borders": true,
                "show_origin_indicator": true
            },
            "background": background,
            "layout": "continuous_vertical"
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

    // Stream the JSON (stroke by stroke) into the gzip writer so that the
    // peak memory stays bounded by the largest single stroke.
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(5));
    write!(
        encoder,
        r#"{{"version":"{}","data":{{"engine_snapshot":{{"document":{},"camera":{},"stroke_components":["#,
        options.rnote_version,
        serde_json::to_string(&document)?,
        serde_json::to_string(&camera)?,
    )?;
    write!(encoder, r#"{{"value":null,"version":0}}"#)?;

    let mut chrono: u32 = 0;
    for (page_idx, page) in nonempty.iter().enumerate() {
        let (dx, dy) = page_offset(page_idx, page);
        for s in page.strokes.iter() {
            let moved = OutStroke {
                points: s
                    .points
                    .iter()
                    .map(|(x, y)| (x + dx, y + dy))
                    .collect(),
                width_px: s.width_px,
                color_rgba: s.color_rgba,
                highlighter: s.highlighter,
            };
            chrono += 1;
            write!(
                encoder,
                ",{}",
                serde_json::to_string(&json!({"value": stroke_json(&moved), "version": 1}))?
            )?;
        }
    }

    write!(encoder, r#"],"chrono_components":[{{"value":null,"version":0}}"#)?;
    let mut t: u32 = 0;
    for page in nonempty.iter() {
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
    }

    write!(encoder, r#"],"chrono_counter":{}"#, chrono)?;
    encoder.write_all(b"}}}")?;

    Ok(encoder.finish()?)
}

/// Convert OneNote ink-space data into output strokes in pixels.
pub fn prepare_strokes(pages: &[crate::onedata::PageData], options: &Options) -> Vec<PreparedPage> {
    pages
        .iter()
        .map(|page| {
            let strokes = page
                .strokes
                .iter()
                .map(|s| convert_stroke(s, options.dpi))
                .collect();
            PreparedPage {
                strokes,
                height_half_inch: page.height_half_inch,
            }
        })
        .collect()
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
