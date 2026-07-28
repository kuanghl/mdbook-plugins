//! mdbook-pdf HTML 预处理模块
//!
//! 在 HTML 送入 Chrome 前进行多维度预处理:
//! - ToC 锚点注入 (PDF 命名目标)
//! - JS 注入 (展开 `<details>`、MathJax 挂钩、内容加载哨兵)
//! - 链接修正 (相对路径 → 绝对 URL)
//! - 打印 CSS 注入 (`@media print` 分页控制)
//! - CJK 字体回退 CSS 注入
//! - Emoji 替换（Noto Emoji SVG，国旗使用 region-flags 子目录补全）

use std::path::Path;
use std::fs;

use scraper::{Html, Selector};

/// 章节路径 → PDF 命名目标 ID
///
/// "chapter/01-setup.md" → "chapter-01-setup"
pub fn chapter_path_to_id(path: &str) -> String {
    let mut base = path.to_string();
    if base.ends_with(".md") {
        base.truncate(base.len() - 3);
    }
    base.replace(['/', '\\'], "-")
        .to_ascii_lowercase()
}

/// 在 `</body>` 前插入隐藏锚点，供 PDF 书签定位
///
/// 每个锚点对应一个章节，Chrome 将其转为 PDF 命名目标。
pub fn inject_toc_fix(html: &str, chapter_paths: &[String]) -> String {
    let mut toc_fix = String::from("<div style=\"display: none\">");
    for path in chapter_paths {
        let id = chapter_path_to_id(path);
        toc_fix.push_str(&format!("<a id=\"{}\"></a>", id));
    }
    toc_fix.push_str("</div>");
    insert_before(html, "</body>", &toc_fix)
}

/// 删除阻塞型外部 CDN 资源（脚本、样式表），保留图片
///
/// `<script>` 和 `<link>` 标签是同步阻塞资源，在 WSL 中加载极慢（数分钟），
/// 会阻塞 HTML 解析和 window.load。删除它们后页面加载不受外部 CDN 影响。
/// `<img>` 图片是并行加载资源，保留以保证 PDF 内容完整（如 emoji SVG）。
pub fn remove_external_resources(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(start) = rest.find('<') {
        result.push_str(&rest[..start]);
        let tag_end = rest[start..].find('>').map(|i| start + i + 1).unwrap_or(rest.len());
        let tag = &rest[start..tag_end];

        let is_external = tag.contains("src=\"http") || tag.contains("src='http")
            || tag.contains("href=\"http") || tag.contains("href='http");

        if is_external {
            // 脚本和样式表是同步阻塞资源，必须删除
            // 图片是并行加载不阻塞 HTML 解析，保留以保证 PDF 内容完整
            if tag.starts_with("<script ") || tag.starts_with("<script>") {
                result.push_str("<script>/* CDN removed for PDF */</script>");
            } else if tag.starts_with("<link ") {
                // link 标签跳过即可
            } else {
                result.push_str(tag);
            }
        } else {
            result.push_str(tag);
        }
        rest = &rest[tag_end..];
    }
    result.push_str(rest);
    result
}

/// 注入 JS 脚本:
/// - 展开所有 `<details>` 元素，移除固定页眉/页脚
/// - 纯本地加载：window.load（所有本地资源就绪）→ 2帧 → 注入哨兵
pub fn inject_js(html: &str) -> String {
    let script = r#"<script type='text/javascript'>
// ── 纯本地加载哨兵 ──
// 外部 CDN 脚本（MathJax、translate.js）已在预处理阶段删除，
// window.load 只等本地资源（图片、字体、本地 JS）加载完成，< 1 秒。
// 这是最可靠的方式：window.load 天然保证所有本地资源完整性。
window.addEventListener('load', function () {
    // 1. DOM 操作
    for (var d of document.getElementsByTagName('details'))
        d.open = true;
    var ph = document.getElementById('mdbook-print-header');
    var pf = document.getElementById('mdbook-print-footer');
    if (ph) ph.remove();
    if (pf) pf.remove();

    // 2. 等 2 帧 — ECharts/Mermaid 等把渲染结果提交到屏幕
    var frameCount = 2;
    function frame() {
        if (--frameCount <= 0) {
            var p = document.createElement('div');
            p.setAttribute('id', 'content-has-all-loaded-for-mdbook-pdf-generation');
            document.body.appendChild(p);
        } else {
            requestAnimationFrame(frame);
        }
    }
    requestAnimationFrame(frame);
});

// MathJax 异步加载兼容：轮询等待 MathJax 就绪后注册钩子
(function () {
    var mjTimer = setInterval(function () {
        try {
            if (window.MathJax && MathJax.Hub) {
                clearInterval(mjTimer);
                MathJax.Hub.Register.StartupHook('End', function () {
                    window.__pdfRenderLatch.push(Promise.resolve());
                });
            }
        } catch (e) {}
    }, 200);
    // 60 秒后停止轮询（防止永久挂起）
    setTimeout(function () { clearInterval(mjTimer); }, 60000);
})();
</script>"#;
    insert_before(html, "</body>", script)
}

/// 修正相对链接为绝对 URL
///
/// 仅当 `base_url` 非空时生效。跳过锚点链接 (`#...`) 和已有协议的链接。
pub fn fix_links(html: &str, base_url: &str) -> String {
    if base_url.is_empty() {
        return html.to_string();
    }
    let base_url = base_url.trim_end_matches('/');
    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href]").unwrap();

    let mut replacements: Vec<(String, String)> = Vec::new();
    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            if let Some(fixed) = fix_single_link(href, base_url) {
                replacements.push((href.to_string(), fixed));
            }
        }
    }

    let mut result = html.to_string();
    for (old, new) in &replacements {
        result = result.replace(
            &format!("href=\"{}\"", old),
            &format!("href=\"{}\"", new),
        );
    }
    result
}

/// 修正单个链接
fn fix_single_link(href: &str, base_url: &str) -> Option<String> {
    // 跳过锚点链接和已有协议的链接
    if href.starts_with('#') || href.starts_with("http://") || href.starts_with("https://") {
        return None;
    }
    // 跳过 mailto: 等协议链接
    if href.contains("://") || href.starts_with("mailto:") {
        return None;
    }
    // 修正相对路径（以 ../ 开头或包含 /../）
    if href.starts_with("../") || href.contains("/../") {
        let clean_href = href.replace('\\', "/");
        let mut fixed = String::new();
        fixed.push_str(base_url);
        if !fixed.ends_with('/') {
            fixed.push('/');
        }
        fixed.push_str(&clean_href);
        return Some(fixed);
    }
    None
}

/// 注入打印 CSS (@media print 分页控制 + 抑制主题打印页眉/页脚)
///
/// 防止代码块、表格、图片在打印时分页断裂。
/// 同时隐藏 `#mdbook-print-header` / `#mdbook-print-footer`，避免与 Chrome
/// 原生 displayHeaderFooter 重复渲染。
pub fn inject_print_css(html: &str) -> String {
    let css = r#"<style>
@media print {
    pre, code, pre code {
        page-break-inside: avoid;
    }
    table {
        page-break-inside: avoid;
    }
    img {
        page-break-inside: avoid;
    }
    h1, h2, h3, h4, h5, h6 {
        page-break-after: avoid;
    }
    a[href]::after {
        content: none !important;
    }
    /* 抑制主题打印页眉/页脚，避免与 Chrome CDP displayHeaderFooter 冲突 */
    #mdbook-print-header, #mdbook-print-footer {
        display: none !important;
    }
}
</style>"#;
    // 插入到 </head> 前，若无则插到 <body> 前
    if html.contains("</head>") {
        insert_before(html, "</head>", css)
    } else if html.contains("<body") {
        insert_before(html, "<body", css)
    } else {
        // 兜底：追加到开头
        format!("{}{}", css, html)
    }
}

/// 注入 CJK 字体回退 CSS，避免方框乱码
pub fn inject_font_css(html: &str) -> String {
    let css = r#"<style>
body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
        "Noto Sans SC", "Microsoft YaHei", "PingFang SC",
        "Hiragino Sans GB", "WenQuanYi Micro Hei",
        "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji",
        "Helvetica Neue", Arial, sans-serif;
}
code, pre {
    font-family: "Cascadia Code", "JetBrains Mono", "Fira Code",
        "Source Code Pro", "Noto Sans Mono CJK SC",
        "Microsoft YaHei Mono", Consolas, monospace;
}
</style>"#;
    if html.contains("</head>") {
        insert_before(html, "</head>", css)
    } else {
        format!("{}{}", css, html)
    }
}

/// CSS 注入页眉/页脚
///
/// 根据配置生成 `position: fixed` 的页眉/页脚 div 和 `@page` 边距补偿。
pub fn inject_css_header_footer(
    html: &str,
    header_content: &str,
    footer_content: &str,
    header_height: f64,
    footer_height: f64,
    margin_top: f64,
    margin_bottom: f64,
    margin_left: f64,
    margin_right: f64,
) -> String {
    let css = format!(
        r#"<style>
@media print {{
    .pf-h, .pf-f {{
        display: block;
        position: fixed;
        left: 0; right: 0; width: 100%;
        z-index: 10000;
        font-size: 10px;
    }}
    .pf-h {{
        top: 0;
        height: {header_height}in;
    }}
    .pf-f {{
        bottom: 0;
        height: {footer_height}in;
    }}
}}
@page {{
    margin: {compensated_mt}in {mr}in {compensated_mb}in {ml}in;
}}
</style>"#,
        header_height = header_height,
        footer_height = footer_height,
        compensated_mt = margin_top + header_height,
        mr = margin_right,
        compensated_mb = margin_bottom + footer_height,
        ml = margin_left,
    );

    let header_div = format!(
        r#"<div class="pf-h">{}</div>"#,
        header_content
    );
    let footer_div = format!(
        r#"<div class="pf-f">{}</div>"#,
        footer_content
    );

    let mut result = if html.contains("</head>") {
        insert_before(html, "</head>", &css)
    } else {
        format!("{}{}", css, html)
    };
    // 在 </body> 前插入页眉/页脚 div
    result = insert_before(&result, "</body>", &header_div);
    result = insert_before(&result, "</body>", &footer_div);
    result
}

/// 在 `target` 字符串前插入 `insertion` 文本
fn insert_before(original: &str, target: &str, insertion: &str) -> String {
    if let Some(pos) = original.find(target) {
        let mut result = String::with_capacity(original.len() + insertion.len());
        result.push_str(&original[..pos]);
        result.push_str(insertion);
        result.push_str(&original[pos..]);
        result
    } else {
        // 如果找不到 target，追加到末尾
        format!("{}{}", original, insertion)
    }
}

/// 将 .pdfviewer-container 替换为静态链接（PDF 输出用）
///
/// pdf_preview 预处理器生成的交互式容器在 PDF 中无法工作，
/// 替换为指向 PDF 文件的 `<a>` 链接以保留可用性。
pub fn replace_pdf_containers(html: &str) -> String {
    let mut result = String::new();
    let mut remaining = html;
    let container_start = r#"<div class="pdfviewer-container" data-pdf-src=""#;

    loop {
        match remaining.find(container_start) {
            None => {
                result.push_str(remaining);
                break;
            }
            Some(pos) => {
                // 保留容器之前的内容
                result.push_str(&remaining[..pos]);

                let after_start = &remaining[pos + container_start.len()..];

                // 提取 data-pdf-src 属性值
                if let Some(quote_end) = after_start.find('"') {
                    let pdf_src = &after_start[..quote_end];

                    // 在剩余部分中查找 ppv-filename
                    let fname_marker = r#"<div class="ppv-filename">"#;
                    let after_src = &after_start[quote_end..];
                    if let Some(fn_pos) = after_src.find(fname_marker) {
                        let after_fn_tag = &after_src[fn_pos + fname_marker.len()..];
                        if let Some(fn_end) = after_fn_tag.find("</div>") {
                            let filename = &after_fn_tag[..fn_end];

                            // 查找容器的闭合标签（</div>\n</div>）
                            let container_close = "</div>\n</div>";
                            if let Some(close_pos) = after_fn_tag[fn_end..].find(container_close) {
                                // 替换为链接
                                result.push_str(&format!(
                                    r#"<a href="{}">📄 {}</a>"#,
                                    pdf_src,
                                    filename
                                ));
                                remaining = &after_fn_tag[fn_end + close_pos + container_close.len()..];
                                continue;
                            }
                        }
                    }
                }

                // 解析失败时保留原始内容
                result.push_str(&remaining[pos..pos + container_start.len()]);
                remaining = &remaining[pos + container_start.len()..];
            }
        }
    }

    result
}

/// Regional Indicator 字符对 → 两字母国家代码
/// 如 🇨🇳 (U+1F1E8 U+1F1F3) → "CN"
fn regional_indicator_to_country_code(emoji_str: &str) -> Option<String> {
    let chars: Vec<char> = emoji_str.chars().filter(|&c| c as u32 != 0xFE0F).collect();
    if chars.len() != 2 {
        return None;
    }
    let c0 = chars[0] as u32;
    let c1 = chars[1] as u32;
    // Regional Indicator Symbols: U+1F1E6 (🇦) ~ U+1F1FF (🇿)
    if c0 >= 0x1F1E6 && c0 <= 0x1F1FF && c1 >= 0x1F1E6 && c1 <= 0x1F1FF {
        let a = |c: u32| -> Option<char> { char::from_u32((c - 0x1F1E6) + ('A' as u32)) };
        Some(format!("{}{}", a(c0)?, a(c1)?))
    } else {
        None
    }
}

/// 将 HTML 中的 emoji 替换为 Noto Emoji SVG 图片标签
///
/// 普通 emoji → `svg/emoji_u{codepoints}.svg`
/// 国旗 emoji → `third_party/region-flags/svg/{CODE}.svg`
pub fn replace_emoji_with_text(html: &str) -> String {
    let mut emojis_list: Vec<&'static str> = emojis::iter().map(|e| e.as_str()).collect();
    emojis_list.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut result = String::new();
    let mut remaining = html;

    'outer: while !remaining.is_empty() {
        for e_str in &emojis_list {
            if remaining.starts_with(e_str) {
                if let Some(country_code) = regional_indicator_to_country_code(e_str) {
                    // 国旗 → third_party/region-flags（唯一存在的国旗 SVG）
                    result.push_str("<img src=\"https://cdn.jsdelivr.net/gh/googlefonts/noto-emoji@main/third_party/region-flags/svg/");
                    result.push_str(&country_code);
                    result.push_str(".svg\" alt=\"emoji\" class=\"noto-emoji\" style=\"height:1em;vertical-align:middle;\">");
                } else {
                    // 普通 emoji → svg/emoji_u{hex}.svg
                    let codepoints: Vec<_> = e_str
                        .chars()
                        .filter(|&c| c as u32 != 0xFE0F)
                        .map(|c| format!("{:x}", c as u32))
                        .collect();
                    result.push_str("<img src=\"https://cdn.jsdelivr.net/gh/googlefonts/noto-emoji@main/svg/emoji_u");
                    result.push_str(&codepoints.join("_"));
                    result.push_str(".svg\" alt=\"emoji\" class=\"noto-emoji\" style=\"height:1em;vertical-align:middle;\">");
                }
                remaining = &remaining[e_str.len()..];
                continue 'outer;
            }
        }
        let ch = remaining.chars().next().unwrap();
        result.push(ch);
        remaining = &remaining[ch.len_utf8()..];
    }

    result
}

pub fn preprocess(
    html: &str,
    chapter_paths: &[String],
    cfg: &super::pdf::PdfOptions,
    book_root: Option<&Path>,
) -> String {
    let mut result = html.to_string();

    // 1. 链接修正
    if !cfg.static_site_url.is_empty() {
        result = fix_links(&result, &cfg.static_site_url);
    }

    // 2. ToC 锚点注入
    result = inject_toc_fix(&result, chapter_paths);

    // 3. 打印 CSS 注入
    result = inject_print_css(&result);

    // 4. CJK 字体 CSS 注入
    result = inject_font_css(&result);

    // 修复 CSS 页眉/页脚注入与原生模板互斥
    let css_hf = cfg.css_header_footer && !cfg.use_native_header_footer;
    if css_hf && cfg.header_footer_enabled() {
        result = inject_css_header_footer(
            &result,
            &cfg.header_template,
            &cfg.footer_template,
            cfg.header_height,
            cfg.footer_height,
            cfg.margin_top,
            cfg.margin_bottom,
            cfg.margin_left,
            cfg.margin_right,
        );
    }

    // 5. 删除阻塞型外部 CDN 资源（脚本、样式表），保留图片
    result = remove_external_resources(&result);

    // 6. JS 注入（始终执行）
    result = inject_js(&result);

    // 7. 替换 PDF 预览容器为静态链接（PDF 中交互式容器无法工作）
    result = replace_pdf_containers(&result);

    // 8. Emoji 字体：自动检测 theme/fonts/ 下的 emoji woff2 文件
    //    直接使用，不复制。无需任何配置。
    if let Some(root) = book_root {
        let fonts_dir = root.join("theme").join("fonts");
        let mut emoji_css = String::new();
        if fonts_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&fonts_dir) {
                let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                files.sort_by_key(|e| e.file_name());
                for entry in &files {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.ends_with(".woff2") && name_str.contains("emoji") {
                        let rel = format!("../../theme/fonts/{}", name_str);
                        emoji_css.push_str(&format!(
                            "@font-face {{ font-family: 'Emoji PDF'; src: url('{}') format('woff2'); }}\n",
                            rel
                        ));
                    }
                }
            }
        }
        if !emoji_css.is_empty() {
            emoji_css.push_str(
                "body { font-family: 'Emoji PDF', 'Noto Color Emoji', 'Apple Color Emoji', 'Segoe UI Emoji', sans-serif; }"
            );
            let css = format!("<style>{}</style>", emoji_css);
            if let Some(pos) = result.rfind("</head>") {
                result.insert_str(pos, &css);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chapter_path_to_id() {
        assert_eq!(chapter_path_to_id("intro.md"), "intro");
        assert_eq!(chapter_path_to_id("chapter/01-setup.md"), "chapter-01-setup");
        assert_eq!(chapter_path_to_id("guide/getting-started.md"), "guide-getting-started");
    }

    #[test]
    fn test_inject_toc_fix_basic() {
        let html = "<html><body>content</body></html>";
        let paths = vec!["intro.md".to_string(), "chapter/01-setup.md".to_string()];
        let result = inject_toc_fix(html, &paths);
        assert!(result.contains(r#"<a id="intro">"#));
        assert!(result.contains(r#"<a id="chapter-01-setup">"#));
        // 插入在 </body> 前
        assert!(result.ends_with("</body></html>") || result.contains("</body></html>"));
    }

    #[test]
    fn test_inject_js_inserts_before_body_end() {
        let html = "<html><body><p>hello</p></body></html>";
        let result = inject_js(html);
        assert!(result.contains("__pdfRenderLatch"));
        assert!(result.contains("content-has-all-loaded-for-mdbook-pdf-generation"));
        assert!(result.contains("<script"));
    }

    #[test]
    fn test_fix_links_with_base_url() {
        let html = r#"<a href="../images/foo.png">img</a>"#;
        let result = fix_links(html, "https://example.com/book");
        assert!(result.contains(r#"href="https://example.com/book/../images/foo.png""#));
    }

    #[test]
    fn test_fix_links_anchor_skipped() {
        let html = r##"<a href="#section">link</a>"##;
        let result = fix_links(html, "https://example.com/book");
        assert_eq!(result, html);
    }

    #[test]
    fn test_fix_links_empty_base() {
        let html = r#"<a href="../page.html">link</a>"#;
        let result = fix_links(html, "");
        assert_eq!(result, html);
    }

    #[test]
    fn test_inject_print_css() {
        let html = "<html><head></head><body>content</body></html>";
        let result = inject_print_css(html);
        assert!(result.contains("@media print"));
        assert!(result.contains("page-break-inside: avoid"));
    }

    #[test]
    fn test_inject_font_css() {
        let html = "<html><head></head><body>content</body></html>";
        let result = inject_font_css(html);
        assert!(result.contains("Noto Sans SC"));
        assert!(result.contains("Microsoft YaHei"));
    }

    #[test]
    fn test_replace_emoji_with_text() {
        let html = "<p>点击 📄 查看</p>";
        let result = replace_emoji_with_text(html);
        assert!(result.contains("noto-emoji@main/svg/emoji_u"));
        assert!(result.contains("1f4c4.svg"));
        assert!(result.contains("<img"));
        assert!(!result.contains("📄"));
    }

    #[test]
    fn test_replace_emoji_with_text_no_emoji() {
        let html = "<p>纯文本内容 123</p>";
        let result = replace_emoji_with_text(html);
        assert_eq!(result, html);
    }

    #[test]
    fn test_replace_emoji_with_text_multiple() {
        let html = "📄🚀😊";
        let result = replace_emoji_with_text(html);
        assert!(result.contains("1f4c4"));
        assert!(result.contains("1f680"));
        assert!(result.contains("1f60a"));
        assert!(!result.contains("📄"));
        assert!(!result.contains("🚀"));
    }

    #[test]
    fn test_replace_country_flag_emoji_with_text() {
        // 🇨🇳 (U+1F1E8 U+1F1F3) 应使用 region-flags 路径
        let html = "<p>🇨🇳</p>";
        let result = replace_emoji_with_text(html);
        assert!(result.contains("region-flags/svg/CN.svg"));
        assert!(result.contains("<img"));
    }

    #[test]
    fn test_replace_non_country_flag_emoji() {
        // 🏁 (U+1F3C1) 普通旗帜使用标准路径
        let html = "<p>🏁</p>";
        let result = replace_emoji_with_text(html);
        assert!(result.contains("svg/emoji_u1f3c1.svg"));
        assert!(result.contains("<img"));
    }

    #[test]
    fn test_chapter_path_to_id_with_special_chars() {
        assert_eq!(chapter_path_to_id("01-Introduction.md"), "01-introduction");
    }

    #[test]
    fn test_insert_before_found() {
        let result = insert_before("hello world", "world", "beautiful ");
        assert_eq!(result, "hello beautiful world");
    }

    #[test]
    fn test_insert_before_not_found() {
        let result = insert_before("hello", "xyz", "extra");
        assert_eq!(result, "helloextra");
    }

    #[test]
    fn test_replace_pdf_containers_basic() {
        let html = r#"<p>some text</p>
<div class="pdfviewer-container" data-pdf-src="./test.pdf">
<div class="ppv-placeholder">
<div class="ppv-icon">📄</div>
<div class="ppv-filename">test.pdf</div>
<div class="ppv-hint">点击加载 PDF 预览</div>
</div>
</div>
<p>more text</p>"#;
        let result = replace_pdf_containers(html);
        assert!(result.contains(r#"<a href="./test.pdf">"#));
        assert!(result.contains("📄"));
        assert!(result.contains("test.pdf"));
        assert!(!result.contains("pdfviewer-container"));
        assert!(!result.contains("ppv-placeholder"));
    }

    #[test]
    fn test_replace_pdf_containers_no_container() {
        let html = r#"<p>no pdf container here</p>"#;
        let result = replace_pdf_containers(html);
        assert_eq!(result, html);
    }

    #[test]
    fn test_replace_pdf_containers_multiple() {
        let html = r#"
<div class="pdfviewer-container" data-pdf-src="./doc1.pdf">
<div class="ppv-placeholder">
<div class="ppv-icon">📄</div>
<div class="ppv-filename">doc1.pdf</div>
<div class="ppv-hint">点击加载 PDF 预览</div>
</div>
</div>
<p>separator</p>
<div class="pdfviewer-container" data-pdf-src="./doc2.pdf">
<div class="ppv-placeholder">
<div class="ppv-icon">📄</div>
<div class="ppv-filename">doc2.pdf</div>
<div class="ppv-hint">点击加载 PDF 预览</div>
</div>
</div>"#;
        let result = replace_pdf_containers(html);
        assert!(result.contains(r#"<a href="./doc1.pdf">"#));
        assert!(result.contains(r#"<a href="./doc2.pdf">"#));
        assert!(!result.contains("pdfviewer-container"));
    }

}
