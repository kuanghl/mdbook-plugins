pub mod engine;
pub mod pdf2svg;

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Convert TikZ LaTeX source code into an SVG string.
///
/// Pipeline: tectonic (XeTeX) → PDF → hayro-svg → SVG
///
/// `cache_dir` specifies where tectonic stores the precompiled format (`.fmt`) cache.
pub fn text2svg_simple(input: &str, cache_dir: &Path) -> Result<String> {
    let pdf_data = engine::tex_to_pdf(input, cache_dir)?;
    let svg = pdf2svg::pdf_to_svg(pdf_data)?;
    Ok(svg)
}

/// Compute SHA256 hash of TikZ content, used for cache key.
pub fn tikz_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Convert TikZ code to SVG, save both intermediate PDF and final SVG to files,
/// return SVG path (relative to html_root).
///
/// - `content`: cleaned TikZ LaTeX source
/// - `images_dir`: absolute path to `src/images/` directory
/// - `rel_prefix`: relative path from the HTML page to `images/` (e.g. `./images/` or `../images/`)
/// - `cache_dir`: tectonic format cache directory (e.g. `{root}/{build_dir}/Tectonic/`)
///
/// Returns the HTML `<img>` tag referencing the saved SVG.
pub fn text2svg_file(
    content: &str,
    images_dir: &Path,
    rel_prefix: &str,
    cache_dir: &Path,
) -> Result<String> {
    let hash = tikz_content_hash(content);
    let svg_filename = format!("{}.svg", hash);
    let pdf_filename = format!("{}.pdf", hash);
    let svg_filepath = images_dir.join(&svg_filename);

    if !svg_filepath.exists() {
        std::fs::create_dir_all(images_dir)
            .map_err(|e| anyhow::anyhow!("failed to create images dir: {}", e))?;

        // Compile TeX → PDF
        let pdf_data = engine::tex_to_pdf(content, cache_dir)?;

        // Save PDF first, then convert to SVG (pdf_to_svg takes ownership to avoid clone)
        std::fs::write(images_dir.join(&pdf_filename), &pdf_data)
            .map_err(|e| anyhow::anyhow!("failed to write PDF file: {}", e))?;

        let svg = pdf2svg::pdf_to_svg(pdf_data)?;
        std::fs::write(&svg_filepath, &svg)
            .map_err(|e| anyhow::anyhow!("failed to write SVG file: {}", e))?;
    }

    Ok(format!(
        r#"<img src="{}{}" alt="TikZ diagram" style="max-width:100%;">"#,
        rel_prefix, svg_filename
    ))
}

/// Compute the relative path prefix from an HTML chapter file to the `images/` directory.
///
/// `chapter_path` is relative to the book's `src/` directory (e.g. `test/7.latex_pictures.md`).
/// Returns e.g. `"../images/"` or `"./images/"`.
pub fn relative_svg_prefix(chapter_path: &Path) -> String {
    let depth = chapter_path.parent().map(|p| p.components().count()).unwrap_or(0);
    if depth == 0 {
        "./images/".to_string()
    } else {
        let parents: Vec<&str> = std::iter::repeat("..").take(depth).collect();
        format!("{}/images/", parents.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_prefix() {
        assert_eq!(relative_svg_prefix(Path::new("index.md")), "./images/");
        assert_eq!(relative_svg_prefix(Path::new("test/7.md")), "../images/");
        assert_eq!(relative_svg_prefix(Path::new("a/b/c.md")), "../../images/");
    }

    #[test]
    fn test_content_hash() {
        let h1 = tikz_content_hash("hello");
        let h2 = tikz_content_hash("hello");
        let h3 = tikz_content_hash("world");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 64); // SHA256 hex
    }
}
