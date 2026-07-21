use anyhow::Result;

/// Convert raw PDF bytes into an SVG string using hayro-svg.
///
/// Takes ownership of `pdf_data` to avoid an extra clone.
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

    Ok(svg)
}
