use anyhow::Result;
use crate::tikz::text_device;
use rayon::prelude::*;

/// Convert each PDF page into an independent SVG string using hayro-svg.
///
/// 分页输出（`Vec<String>`，每页一个独立 SVG）：
/// - **单页**：返回 `vec![svg]`，输出结构与旧版一致（缓存文件不变）；
/// - **多页**（完整 LaTeX 文档，如 article/report 类）：每页转为一个独立 SVG
///   并用 rayon **并行**渲染——避免合并成一张超大单文件（XML 解析压力大、
///   一页出错连累整图），每页是独立文档，天然没有跨页 id 冲突。
///
/// 每页 SVG：视觉层由 hayro-svg 渲染（路径轮廓）；随后做第二遍
/// `interpret_page` 收集字形文本，生成透明的 `<text>` 层追加到 SVG 末尾，
/// 使图内文字可选中/可搜索。（`<img>` 引用时 text 层透明不可见，无视觉影响。）
///
/// 注：hayro-svg 渲染 LaTeX Type1 字形时，字母内孔（如 P/o/e）无法镂空
/// （渲染为实心），这是 hayro 库的固有行为，不影响可读性。
pub(crate) fn pdf_to_svg_pages(pdf_data: Vec<u8>) -> Result<Vec<String>> {
    let pdf = hayro_syntax::Pdf::new(pdf_data)
        .map_err(|e| anyhow::anyhow!("failed to parse PDF: {:?}", e))?;

    let pages = pdf.pages();
    if pages.is_empty() {
        anyhow::bail!("PDF has no pages");
    }

    // 并行转换每页（RenderCache 每页独立，无共享状态）
    let svgs: Vec<Result<String>> = pages.par_iter().map(|page| render_page(page)).collect();
    svgs.into_iter().collect()
}

/// 单页 PDF → SVG：视觉层 + 透明 text 层直接追加在 svg 末尾。
fn render_page(page: &hayro_syntax::page::Page<'_>) -> Result<String> {
    let cache = hayro_svg::RenderCache::new();
    let interpreter_settings = hayro_interpret::InterpreterSettings::default();
    // Transparent background so TikZ diagrams blend into the HTML page
    let render_settings = hayro_svg::SvgRenderSettings {
        bg_color: [255, 255, 255, 0], // R, G, B, A — fully transparent
    };

    let svg = hayro_svg::convert(page, &cache, &interpreter_settings, &render_settings);

    // 第二遍解释：收集文本层（透明 <text>，用于选中/搜索）
    let mut svg = svg;
    match text_device::collect_page_text_layer(page, &interpreter_settings) {
        Ok(text_layer) if !text_layer.is_empty() => {
            if let Some(pos) = svg.rfind("</svg>") {
                svg.insert_str(pos, &format!("\n{}", text_layer));
            }
        }
        Ok(_) => {}
        Err(e) => log::warn!("TikZ 文本层收集失败（仅影响文字选中/搜索）: {}", e),
    }

    Ok(svg)
}
