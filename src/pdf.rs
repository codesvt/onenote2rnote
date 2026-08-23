use anyhow::Context;
use std::fs;
use std::process::{Command, Stdio};

/// Render a single PDF page to PNG bytes using the system `pdftoppm` (poppler-utils).
///
/// `page_index` is 0-based; pdftoppm expects a 1-based `-f`/`-l` range. The rendered PNG is
/// returned as raw encoded bytes (not decoded).
pub fn render_pdf_page(
    pdf_bytes: &[u8],
    page_index: u32,
    dpi: u32,
) -> anyhow::Result<Vec<u8>> {
    if which_pdftoppm().is_none() {
        anyhow::bail!(
            "`pdftoppm` was not found on your system.\n\
             It is part of poppler-utils; install it, e.g.:\n\
               sudo apt install poppler-utils"
        );
    }

    // pdftoppm cannot read from stdin and only writes PNGs to real files, so we use a
    // unique temporary base path for both the input PDF and the output PNG.
    let base = temp_base();
    let pdf_path = base.with_extension("pdf");
    let png_path = base.with_extension("png");
    fs::write(&pdf_path, pdf_bytes).context("writing temporary PDF failed")?;

    let page = (page_index + 1) as u32;
    let child = Command::new("pdftoppm")
        .arg("-png")
        .arg("-f")
        .arg(page.to_string())
        .arg("-l")
        .arg(page.to_string())
        .arg("-r")
        .arg(dpi.to_string())
        .arg("-singlefile")
        .arg(&pdf_path)
        .arg(&base)
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning pdftoppm failed")?;

    let output = child.wait_with_output().context("waiting for pdftoppm failed")?;
    let _ = fs::remove_file(&pdf_path);
    if !output.status.success() {
        anyhow::bail!(
            "pdftoppm failed (page {page}, dpi {dpi}): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let png = fs::read(&png_path).context("reading rendered PNG failed")?;
    let _ = fs::remove_file(&png_path);
    if png.is_empty() {
        anyhow::bail!("pdftoppm produced no output for page {page}");
    }

    Ok(png)
}

/// Merge several PDFs into one (in order) using the system `pdfunite` (poppler-utils).
///
/// OneNote stores an inserted multi-page PDF as one single-page PDF per page, so merging them
/// reconstructs the original document. Returns the merged PDF bytes.
pub fn merge_pdfs(pdfs: &[&[u8]]) -> anyhow::Result<Vec<u8>> {
    match pdfs.len() {
        0 => anyhow::bail!("no PDFs to merge"),
        1 => return Ok(pdfs[0].to_vec()),
        _ => {}
    }
    if which_pdfunite().is_none() {
        anyhow::bail!(
            "`pdfunite` was not found on your system.\n\
             It is part of poppler-utils; install it, e.g.:\n\
               sudo apt install poppler-utils"
        );
    }

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("onenote2rnote-{}-{}", std::process::id(), n));
    let out_path = base.with_extension("merged.pdf");

    let mut cmd = Command::new("pdfunite");
    for (i, bytes) in pdfs.iter().enumerate() {
        let path = base.with_extension(format!("in-{i}.pdf"));
        fs::write(&path, bytes).context("writing temporary PDF failed")?;
        cmd.arg(&path);
    }
    cmd.arg(&out_path);

    let child = cmd
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning pdfunite failed")?;
    let output = child.wait_with_output().context("waiting for pdfunite failed")?;
    if !output.status.success() {
        anyhow::bail!(
            "pdfunite failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let merged = fs::read(&out_path).context("reading merged PDF failed")?;
    let _ = fs::remove_file(&out_path);
    for i in 0..pdfs.len() {
        let _ = fs::remove_file(base.with_extension(format!("in-{i}.pdf")));
    }
    if merged.is_empty() {
        anyhow::bail!("pdfunite produced no output");
    }
    Ok(merged)
}

/// Best-effort check whether `pdftoppm` is on PATH.
pub fn pdftoppm_available() -> bool {
    which_pdftoppm().is_some()
}

fn which_pdfunite() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("pdfunite");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn which_pdftoppm() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("pdftoppm");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn temp_base() -> std::path::PathBuf {
    // Use a predictable unique base name in the system temp dir.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("onenote2rnote-{}-{}", std::process::id(), n))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal one-page PDF (200x200 pt) used to exercise `pdftoppm`.
    const MINI_PDF: &str = "JVBERi0xLjQKMSAwIG9iago8PCAvTGVuZ3RoIDIwID4+CnN0cmVhbQpxIDAgMCAyMDAgMjAwIHJlIGYgUQplbmRzdHJlYW0KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCAyMDAgMjAwXSAvQ29udGVudHMgMSAwIFIgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFsyIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL1R5cGUgL0NhdGFsb2cgL1BhZ2VzIDMgMCBSID4+CmVuZG9iagp4cmVmCjAgNQowMDAwMDAwMDAwIDY1NTM1IGYgCjAwMDAwMDAwMDkgMDAwMDAgbiAKMDAwMDAwMDA3OSAwMDAwMCBuIAowMDAwMDAwMTY2IDAwMDAwIG4gCjAwMDAwMDAyMjMgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA1IC9Sb290IDQgMCBSID4+CnN0YXJ0eHJlZgoyNzIKJSVFT0YK";

    fn mini_pdf() -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(MINI_PDF)
            .expect("valid base64")
    }

    #[test]
    fn renders_pdf_page_to_png_when_pdftoppm_present() {
        if !pdftoppm_available() {
            eprintln!("skipping: pdftoppm not installed");
            return;
        }
        let png = render_pdf_page(&mini_pdf(), 0, 96).expect("render page 0");
        assert!(!png.is_empty(), "expected non-empty PNG");
        // PNG magic bytes.
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn merges_single_page_pdfs_into_one_when_pdfunite_present() {
        if which_pdfunite().is_none() {
            eprintln!("skipping: pdfunite not installed");
            return;
        }
        let pdf = mini_pdf();
        let merged = merge_pdfs(&[&pdf, &pdf]).expect("merge two pages");
        assert!(!merged.is_empty());
        assert!(merged.starts_with(b"%PDF"));
        // A merged two-page PDF should contain two /Type /Page objects (but only one /Type /Pages).
        let txt = String::from_utf8_lossy(&merged);
        assert_eq!(txt.matches("/Type /Page ").count(), 2);
    }

    #[test]
    fn detects_missing_pdftoppm() {
        // We only assert the shape of the error when the tool is absent; the system may have it.
        if !pdftoppm_available() {
            let err = render_pdf_page(b"%PDF-1.4 fake", 0, 96).unwrap_err();
            assert!(err.to_string().contains("pdftoppm"));
        }
    }
}
