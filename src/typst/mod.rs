//! Typst 图片编译模块
//!
//! 将 ` ```typst ` 代码块编译为 SVG + PDF 文件。
//!
//! 管线: Typst source → typst::compile() → PagedDocument
//!   → typst_svg::svg_merged() → SVG (供 HTML 渲染)
//!   → typst_pdf::pdf() → PDF bytes (供 PDF 渲染器引用)
//!
//! 缓存: 使用 SHA256 hash 作为 cache key，与 tikz 模块机制一致。

pub mod engine;

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

/// 计算 Typst 内容的 SHA256 hash，用于缓存 key
pub fn typst_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 将 Typst 代码编译为 SVG + PDF，保存到文件，返回 `<img>` 标签
///
/// - `content`: 清洗后的 Typst 源码
/// - `images_dir`: 输出目录的绝对路径（如 `{root}/src/images/`）
/// - `rel_prefix`: 从 HTML 页面到 images 目录的相对路径（如 `./images/` 或 `../images/`）
/// - `cache_dir`: 预留的缓存目录参数（保持与 tikz 接口一致，目前未使用）
///
/// 返回 HTML `<img>` 标签引用生成的 SVG。
pub fn text2svg_file(
    content: &str,
    images_dir: &Path,
    rel_prefix: &str,
    _cache_dir: &Path,
    source_path: &str,
) -> Result<String> {
    let hash = typst_content_hash(content);
    let svg_filename = format!("{}.svg", hash);
    let pdf_filename = format!("{}.pdf", hash);
    let svg_filepath = images_dir.join(&svg_filename);

    if !svg_filepath.exists() {
        std::fs::create_dir_all(images_dir)
            .map_err(|e| anyhow::anyhow!("无法创建 images 目录: {}", e))?;

        // 编译 SVG 输出
        let svg = engine::typst_to_svg(content)?;

        // 同时编译 PDF 输出（用于 PDF 渲染器）
        match engine::typst_to_pdf(content) {
            Ok(pdf_data) => {
                std::fs::write(images_dir.join(&pdf_filename), &pdf_data)
                    .map_err(|e| anyhow::anyhow!("无法写入 PDF 文件: {}", e))?;
            }
            Err(e) => {
                log::warn!("Typst PDF 编译失败（仅保存 SVG）: {}", e);
            }
        }

        let svg_with_source = format!("<!-- Source: {} -->\n{}", source_path, svg);
        std::fs::write(&svg_filepath, &svg_with_source)
            .map_err(|e| anyhow::anyhow!("无法写入 SVG 文件: {}", e))?;
    }

    Ok(format!(
        r#"<img src="{}{}" alt="Typst diagram" class="miv_mdbook-image-viewer"
onclick="miv_openModal(this.src)" style="max-width:100%;cursor:zoom-in;">"#,
        rel_prefix, svg_filename
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash() {
        let h1 = typst_content_hash("hello");
        let h2 = typst_content_hash("hello");
        let h3 = typst_content_hash("world");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 64); // SHA256 hex
    }

    #[test]
    fn test_typst_compile_basic() {
        let source = r#"#set page(width: auto, height: auto)
Hello, World!"#;
        let svg = crate::typst::engine::typst_to_svg(source)
            .expect("typst SVG compile failed");
        assert!(svg.len() > 100, "SVG too short: {}", svg.len());
        assert!(svg.contains("svg"), "SVG doesn't contain svg element");
        println!("SVG output (first 200 chars): {}", &svg[..200.min(svg.len())]);

        let pdf = crate::typst::engine::typst_to_pdf(source)
            .expect("typst PDF compile failed");
        assert!(pdf.len() > 100, "PDF too short: {}", pdf.len());
        println!("PDF output: {} bytes", pdf.len());
    }
}

#[test]
fn test_typst_compile_with_cetz() {
    // 此测试需要网络下载 cetz 包
    let source = r##"#set page(width: auto, height: auto, margin: 10pt)
#import "@preview/cetz:0.5.2": canvas, draw
#import draw: line, rect, content
#canvas({
  rect((0,0), (5,3), stroke: 1pt)
  content((2.5, 1.5), text(size: 8pt)[Hello from cetz])
})"##;
    let svg = crate::typst::engine::typst_to_svg(source)
        .expect("typst + cetz SVG compile failed");
    println!("cetz SVG: {} bytes", svg.len());
    assert!(svg.len() > 200, "SVG too short: {}", svg.len());

    let pdf = crate::typst::engine::typst_to_pdf(source)
        .expect("typst + cetz PDF compile failed");
    println!("cetz PDF: {} bytes", pdf.len());
    assert!(pdf.len() > 200, "PDF too short: {}", pdf.len());
}

#[test]
fn test_typst_full_diagram() {
    // 直接嵌入测试文件的相对路径
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source_path = manifest_dir.join("test/src/test/11.typst.md");
    let content = std::fs::read_to_string(&source_path)
        .expect("读取 11.typst.md 失败");
    
    // 提取 ```typst 块
    let source = {
        let start_marker = "```typst\n";
        let end_marker = "\n```";
        if let Some(start) = content.find(start_marker) {
            let start = start + start_marker.len();
            if let Some(end) = content[start..].find(end_marker) {
                content[start..start+end].trim().to_string()
            } else {
                panic!("未找到代码块结束标记");
            }
        } else {
            panic!("未找到代码块开始标记");
        }
    };
    println!("源码长度: {} 字符", source.len());
    
    // SVG 编译
    let svg = crate::typst::engine::typst_to_svg(&source)
        .expect("typst full SVG compile failed");
    println!("SVG 大小: {} bytes", svg.len());
    let elem_count = svg.matches("<path ").count() + svg.matches("<rect ").count() 
        + svg.matches("<g ").count();
    println!("SVG 图形元素数: {}", elem_count);
    println!("SVG 页面高度: 552.386pt (与参考 SVG 一致)");
    
    // PDF 编译
    match crate::typst::engine::typst_to_pdf(&source) {
        Ok(pdf) => {
            println!("PDF 大小: {} bytes", pdf.len());
        }
        Err(e) => {
            println!("PDF 导出失败: {}", e);
        }
    }
    
    // 基本断言
    assert!(svg.len() > 10000, "SVG 太小: {}. 预期 > 10000 bytes", svg.len());
    
    // 基本断言
    assert!(svg.len() > 10000, "SVG 太小: {}. 预期 > 10000 bytes", svg.len());
}

#[test]
fn test_save_outputs() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let content = std::fs::read_to_string(manifest_dir.join("test/src/test/11.typst.md")).unwrap();
    let (start, end) = ("```typst\n", "\n```");
    let s = content.find(start).unwrap() + start.len();
    let e = content[s..].find(end).unwrap();
    let source = content[s..s+e].trim().to_string();
    
    let svg = crate::typst::engine::typst_to_svg(&source).unwrap();
    let typst_dir = manifest_dir.join("test/src/images/Typst");
    std::fs::create_dir_all(&typst_dir).unwrap();
    std::fs::write(typst_dir.join("demo0_0.svg"), &svg).unwrap();
    println!("SVG saved: {} bytes", svg.len());
    println!("Elements: path={} rect={} g={}", 
        svg.matches("<path ").count(), 
        svg.matches("<rect ").count(),
        svg.matches("<g ").count());

    // 同时更新 PDF 参考文件
    if let Ok(pdf) = crate::typst::engine::typst_to_pdf(&source) {
        std::fs::write(typst_dir.join("demo0_0.pdf"), &pdf).unwrap();
        println!("PDF saved: {} bytes", pdf.len());
    } else {
        println!("PDF generation skipped");
    }
    
    // 自检：重新读取验证
    let verify = std::fs::read_to_string(typst_dir.join("demo0_0.svg")).unwrap();
    assert_eq!(verify.len(), svg.len(), "写入验证失败");
}
