pub mod engine;
pub mod pdf2svg;
pub mod text_device;

use anyhow::Result;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Compute SHA256 hash of TikZ content, used for cache key.
pub fn tikz_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 编译 TikZ → SVG（+中间 PDF），写入缓存文件，返回 (每页 SVG 字符串列表, 内容 hash)。
///
/// - `content`: cleaned TikZ LaTeX source
/// - `images_dir`: absolute path to `src/images/` directory
/// - `cache_dir`: tectonic format cache directory (e.g. `{root}/{build_dir}/Tectonic/`)
///
/// 命中缓存（单页 `{hash}.svg` 或多页 `{hash}.p1.svg` 已存在）时直接读取，
/// 不再重新编译。多页（完整 LaTeX 文档）每页一个独立 SVG 文件
/// （`{hash}.p{i}.svg`），避免超大单文件。
fn compile_and_cache(
    content: &str,
    images_dir: &Path,
    cache_dir: &Path,
    source_path: &str,
) -> Result<(Vec<String>, String)> {
    let hash = tikz_content_hash(content);
    let pdf_filename = format!("{}.pdf", hash);
    let pdf_filepath = images_dir.join(&pdf_filename);
    let svg_filepath = images_dir.join(format!("{}.svg", hash));
    let page1_filepath = images_dir.join(format!("{}.p1.svg", hash));

    if !svg_filepath.exists() && !page1_filepath.exists() {
        std::fs::create_dir_all(images_dir)
            .map_err(|e| anyhow::anyhow!("failed to create images dir: {}", e))?;

        // Compile TeX → PDF
        let pdf_data = engine::tex_to_pdf(content, cache_dir)?;

        // Save intermediate PDF
        std::fs::write(&pdf_filepath, &pdf_data)
            .map_err(|e| anyhow::anyhow!("failed to write PDF file: {}", e))?;

        // 分页转 SVG（内部并行）
        let svgs = pdf2svg::pdf_to_svg_pages(pdf_data)?;
        if svgs.len() == 1 {
            let svg_with_source = format!("<!-- Source: {} -->\n{}", source_path, svgs[0]);
            std::fs::write(&svg_filepath, &svg_with_source)
                .map_err(|e| anyhow::anyhow!("failed to write SVG file: {}", e))?;
        } else {
            for (i, svg) in svgs.iter().enumerate() {
                let svg_with_source =
                    format!("<!-- Source: {} (page {}) -->\n{}", source_path, i + 1, svg);
                let path = images_dir.join(format!("{}.p{}.svg", hash, i + 1));
                std::fs::write(&path, &svg_with_source)
                    .map_err(|e| anyhow::anyhow!("failed to write SVG page file: {}", e))?;
            }
        }
    }

    // 读取：单页 `{hash}.svg` 或多页 `{hash}.p{i}.svg`（按页号排序）
    let svgs = if svg_filepath.exists() {
        vec![std::fs::read_to_string(&svg_filepath)
            .map_err(|e| anyhow::anyhow!("failed to read SVG file: {}", e))?]
    } else {
        let re = Regex::new(&format!(r"^{}\.p(\d+)\.svg$", regex::escape(&hash))).unwrap();
        let mut pages: Vec<(usize, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(images_dir)
            .map_err(|e| anyhow::anyhow!("failed to read images dir: {}", e))?
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(cap) = re.captures(&name) {
                if let Ok(n) = cap[1].parse::<usize>() {
                    pages.push((n, entry.path()));
                }
            }
        }
        pages.sort_by_key(|(n, _)| *n);
        pages
            .into_iter()
            .map(|(_, p)| {
                std::fs::read_to_string(&p).map_err(|e| {
                    anyhow::anyhow!("failed to read SVG page file {}: {}", p.display(), e)
                })
            })
            .collect::<Result<Vec<_>>>()?
    };

    Ok((svgs, hash))
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
    alt: &str,
) -> Result<String> {
    let (svgs, hash) = compile_and_cache(content, images_dir, cache_dir, source_path)?;
    // 单页一个 <img>；多页每页一个 <img>（垂直排列）
    let mut out = String::new();
    for (i, _svg) in svgs.iter().enumerate() {
        let filename = if svgs.len() == 1 {
            format!("{}.svg", hash)
        } else {
            format!("{}.p{}.svg", hash, i + 1)
        };
        out.push_str(&format!(
            r#"<img src="{}{}" alt="{}" class="miv_mdbook-image-viewer"
onclick="miv_openModal(this.src)" style="max-width:100%;cursor:zoom-in;">"#,
            rel_prefix, filename, alt
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
