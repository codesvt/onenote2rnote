use flate2::read::GzDecoder;
use onenote2rnote::onedata;
use onenote2rnote::rnote::{FormatKind, Options, prepare_strokes};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;

#[test]
fn real_sample_extracts_ink_and_builds_valid_rnote_structure() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/samples/desktop_missing_ink.one");
    if !sample.exists() {
        eprintln!("skipping: sample file not present");
        return;
    }

    let pages = onedata::parse_input(&sample).expect("parse section");
    assert!(
        pages.iter().any(|p| p.has_ink()),
        "expected ink in desktop_missing_ink.one"
    );
    let total: usize = pages.iter().map(|p| p.strokes.len()).sum();
    assert!(total > 0);

    let options = Options {
        format: FormatKind::A4,
        ..Default::default()
    };
    let prepared = prepare_strokes(&pages, &options);
    let bytes = onenote2rnote::rnote::build_rnote_bytes(&prepared, &options).expect("build");

    let mut json = String::new();
    GzDecoder::new(&bytes[..])
        .read_to_string(&mut json)
        .expect("gunzip");
    let root: Value = serde_json::from_str(&json).expect("json");

    // wrapper
    assert_eq!(root["version"], options.rnote_version);
    let snapshot = &root["data"]["engine_snapshot"];

    // required top-level keys
    for key in [
        "document",
        "camera",
        "stroke_components",
        "chrono_components",
        "chrono_counter",
    ] {
        assert!(snapshot.get(key).is_some(), "missing engine key {key}");
    }

    let sc = snapshot["stroke_components"].as_array().expect("slotmap array");
    let cc = snapshot["chrono_components"]
        .as_array()
        .expect("chrono slotmap array");
    assert_eq!(sc.len(), cc.len(), "stroke and chrono slotmaps must align");

    // slotmap invariants: slot 0 is the empty sentinel, occupied slots have odd versions
    assert_eq!(sc[0], json!({"value": null, "version": 0}));
    assert_eq!(cc[0], json!({"value": null, "version": 0}));

    let mut strokes = 0usize;
    for (i, slot) in sc.iter().enumerate().skip(1) {
        let version = slot["version"].as_u64().expect("version");
        assert_eq!(version % 2, 1, "occupied slot {i} needs an odd version");
        let stroke = &slot["value"];
        assert!(stroke.get("brushstroke").is_some(), "expected brushstroke");

        let path = &stroke["brushstroke"]["path"];
        assert!(path.get("start").is_some());
        let start = &path["start"];
        assert!(start.get("pos").is_some() && start.get("pressure") == Some(&json!(1.0)));
        let segments = path["segments"].as_array().expect("segments");
        assert!(!segments.is_empty(), "stroke {i} has no segments");

        // collect finish points to ensure some geometry exists
        for seg in segments {
            assert!(seg.get("lineto").is_some(), "expected lineto segment");
            let end = &seg["lineto"]["end"];
            let pos = end["pos"].as_array().expect("pos array");
            assert_eq!(pos.len(), 2);
        }

        let style = &stroke["brushstroke"]["style"]["smooth"];
        assert!(style.get("stroke_width").is_some());
        assert!(style.get("stroke_color").is_some());
        assert_eq!(style["pressure_curve"], "const");
        assert_eq!(style["line_cap"], "rounded");
        assert_eq!(style["line_style"], "solid");

        let clock = &cc[i]["value"];
        assert!(clock.get("t").is_some());
        assert!(clock.get("layer").is_some());

        strokes += 1;
    }
    assert_eq!(strokes, total, "every ink stroke must become a component");

    assert_eq!(
        snapshot["chrono_counter"].as_u64().unwrap() as usize,
        strokes
    );

    // document config sanity
    let doc = &snapshot["document"];
    assert!(doc["width"].as_f64().unwrap() > 0.0);
    assert!(doc["height"].as_f64().unwrap() > 0.0);
    let format = &doc["config"]["format"];
    assert!(format["width"].as_f64().unwrap() > 0.0);
    assert!(format["height"].as_f64().unwrap() > 0.0);
    assert!(format["dpi"].as_f64().unwrap() > 0.0);
    assert!(doc["config"]["layout"].is_string());
}

#[test]
fn empty_input_errors() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/samples/deleted_pages.one");
    if !sample.exists() {
        eprintln!("skipping: sample file not present");
        return;
    }
    let pages = onedata::parse_input(&sample).expect("parse section");
    let options = Options::default();
    let prepared = prepare_strokes(&pages, &options);
    let result = onenote2rnote::rnote::build_rnote_bytes(&prepared, &options);
    assert!(result.is_err(), "expected error when no ink is present");
}
