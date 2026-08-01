pub mod engine;
pub mod pdf2svg;
pub mod text_device;

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

/// 编译 TikZ → SVG（+中间 PDF），写入缓存文件，返回 (SVG 字符串, 内容 hash)。
///
/// - `content`: cleaned TikZ LaTeX source
/// - `images_dir`: absolute path to `src/images/` directory
/// - `cache_dir`: tectonic format cache directory (e.g. `{root}/{build_dir}/Tectonic/`)
///
/// 命中缓存（`{hash}.svg` 已存在）时直接读取，不再重新编译。
fn compile_and_cache(
    content: &str,
    images_dir: &Path,
    cache_dir: &Path,
    source_path: &str,
) -> Result<(String, String)> {
    let hash = tikz_content_hash(content);
    let svg_filename = format!("{}.svg", hash);
    let pdf_filename = format!("{}.pdf", hash);
    let svg_filepath = images_dir.join(&svg_filename);

    if !svg_filepath.exists() {
        std::fs::create_dir_all(images_dir)
            .map_err(|e| anyhow::anyhow!("failed to create images dir: {}", e))?;

        // Compile TeX → PDF
        let pdf_data = engine::tex_to_pdf(content, cache_dir)?;

        // Save intermediate PDF
        std::fs::write(images_dir.join(&pdf_filename), &pdf_data)
            .map_err(|e| anyhow::anyhow!("failed to write PDF file: {}", e))?;

        let svg = pdf2svg::pdf_to_svg(pdf_data)?;
        let svg_with_source = format!("<!-- Source: {} -->\n{}", source_path, svg);
        std::fs::write(&svg_filepath, &svg_with_source)
            .map_err(|e| anyhow::anyhow!("failed to write SVG file: {}", e))?;
    }

    let svg = std::fs::read_to_string(&svg_filepath)
        .map_err(|e| anyhow::anyhow!("failed to read SVG file: {}", e))?;
    Ok((svg, hash))
}

/// Convert TikZ code to an inline SVG string (no `<img>` / file reference).
///
/// 编译并缓存 SVG/PDF 文件（PDF 渲染器与图片放大仍依赖文件），返回可直接
/// 嵌入 HTML 的 `<svg>…</svg>`：已剥离 Source 注释、压缩空行、在根元素注入
/// 响应式 + 字体隔离样式（见 [`crate::utils::svg_to_inline`]）。
pub fn text2svg_inline(
    content: &str,
    images_dir: &Path,
    cache_dir: &Path,
    source_path: &str,
) -> Result<String> {
    let (svg, _hash) = compile_and_cache(content, images_dir, cache_dir, source_path)?;
    Ok(crate::utils::svg_to_inline(&svg))
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
    source_path: &str,
) -> Result<String> {
    let (_, hash) = compile_and_cache(content, images_dir, cache_dir, source_path)?;
    let svg_filename = format!("{}.svg", hash);

    Ok(format!(
        r#"<img src="{}{}" alt="TikZ diagram" class="miv_mdbook-image-viewer"
onclick="miv_openModal(this.src)" style="max-width:100%;cursor:zoom-in;">"#,
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
