use anyhow::Result;
use crate::tikz::text_device;

/// Convert raw PDF bytes into an SVG string using hayro-svg.
///
/// Takes ownership of `pdf_data` to avoid an extra clone.
///
/// 视觉层由 hayro-svg 渲染（路径轮廓）；随后做第二遍 `interpret_page` 收集
/// 字形文本，生成透明的 `<text>` 层追加到 SVG 末尾，使图内文字可选中/可搜索。
/// （TikZ 图以 `<img>` 引用 svg 文件时 text 层透明不可见，无视觉影响；
/// 若将来改回内联，text 层即可直接提供文字选中/搜索能力。）
///
/// 注：hayro-svg 渲染 LaTeX Type1 字形时，字母内孔（如 P/o/e）无法镂空
/// （渲染为实心），这是 hayro 库的固有行为，不影响可读性。
pub(crate) fn pdf_to_svg(pdf_data: Vec<u8>) -> Result<String> {
    let pdf = hayro_syntax::Pdf::new(pdf_data)
        .map_err(|e| anyhow::anyhow!("failed to parse PDF: {:?}", e))?;

    let pages = pdf.pages();
    let page = pages
        .first()
        .ok_or_else(|| anyhow::anyhow!("PDF has no pages"))?;

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
