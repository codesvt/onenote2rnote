use crate::onedata::PageData;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// On-disk manifest used for incremental updates.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub pages: BTreeMap<String, PageEntry>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PageEntry {
    pub fingerprint: String,
    /// Output files produced for this page (the `.rnote` and any media sidecars).
    pub filenames: Vec<String>,
}

impl Manifest {
    pub fn load(path: &Path) -> Manifest {
        match fs::read(path).ok() {
            Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            None => Manifest {
                version: 1,
                pages: BTreeMap::new(),
            },
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json).map_err(Into::into)
    }

    /// The set of GUIDs known to the manifest but absent from the given pages (orphaned).
    pub fn orphaned<'a>(&self, present: impl Iterator<Item = &'a str>) -> Vec<String> {
        let present: std::collections::HashSet<&str> = present.collect();
        self.pages
            .keys()
            .filter(|g| !present.contains(g.as_str()))
            .cloned()
            .collect()
    }
}

/// Compute a stable fingerprint of a page's content (strokes + media + metadata).
pub fn fingerprint_page(page: &PageData) -> String {
    let mut h = Sha256::new();
    h.update(page.title.as_deref().unwrap_or("").as_bytes());
    h.update(page.updated_time.to_le_bytes());
    h.update(page.height_half_inch.unwrap_or(0.0).to_le_bytes());

    for s in &page.strokes {
        h.update(s.width_ink.to_le_bytes());
        h.update(s.color.unwrap_or(0).to_le_bytes());
        h.update([s.transparency.unwrap_or(0)]);
        h.update(s.off_half_inch.0.to_le_bytes());
        h.update(s.off_half_inch.1.to_le_bytes());
        for (x, y) in &s.points {
            h.update(x.to_le_bytes());
            h.update(y.to_le_bytes());
        }
    }

    for m in &page.media {
        h.update([m.kind as u8]);
        h.update(m.x_half_inch.to_le_bytes());
        h.update(m.y_half_inch.to_le_bytes());
        h.update(m.width_half_inch.to_le_bytes());
        h.update(m.height_half_inch.to_le_bytes());
        h.update(m.filename.as_bytes());
        h.update(m.page_index.unwrap_or(0).to_le_bytes());
        h.update(&m.bytes);
    }

    hex_encode(h.finalize().as_slice())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").expect("writing to string cannot fail");
    }
    s
}
