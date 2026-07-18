//! mdbook-pdf — HTML 预处理模块
//!
//! 在将 HTML 送入 Chrome CDP 打印之前，执行以下预处理：
//!
//! 1. **ToC 修复**：为每个章节插入隐藏的 `<a>` 锚点，Chrome 将其转为 PDF 命名目标
//! 2. **JS 注入**：扩展 `<details>` 元素、挂钩 MathJax 完成、添加内容加载哨兵
//! 3. **链接修正**：当 `static-site-url` 非空时，将相对路径链接转为绝对 URL
//! 4. **分页保护 CSS**：注入 `@media print` 规则，确保代码块不断页、标题不孤行
//!
//! 使用 `scraper` crate 进行 DOM 解析（而非字符串正则），以正确处理 HTML 结构。

use scraper::{Html, Selector};

use super::pdf::PdfOptions;

/// 预处理后的结果
pub struct PreprocessResult {
    pub html: String,
    /// 是否注入了内容加载哨兵（告诉 CDP 等待此元素）
    pub has_content_sentinel: bool,
}

/// HTML 预处理入口
///
/// # 参数
/// - `html`: 原始 print.html 内容
/// - `cfg`: PDF 配置
/// - `chapter_paths`: 书籍章节路径列表（用于 ToC 修复，可选）
pub fn preprocess_html(
    html: &str,
    cfg: &PdfOptions,
    chapter_paths: &[String],
) -> PreprocessResult {
    let html = fix_links(html, &cfg.static_site_url);
    let html = inject_toc_fix(&html, chapter_paths);
    let html = inject_print_css(&html);
    let html = inject_font_css(&html);
    let (html, has_sentinel) = inject_js(&html);
    PreprocessResult {
        html,
        has_content_sentinel: has_sentinel,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 1. ToC 修复 — 为 PDF 书签创建命名目标
// ─────────────────────────────────────────────────────────────────────────

/// 为每个章节插入隐藏的 `<a>` 锚点元素。
///
/// Chrome 在生成 PDF 时，会为带有 `id` 属性的元素创建命名目标（named destinations）。
/// 这些命名目标是后续书签（outline）引用的关键。
///
/// 每个章节的 `id` 由其路径生成：`path/to/chapter.md` → `path-to-chapter`
fn inject_toc_fix(html: &str, chapter_paths: &[String]) -> String {
    if chapter_paths.is_empty() {
        return html.to_string();
    }

    let mut toc_fix = String::from("<div style=\"display: none\">");
    for path in chapter_paths {
        let print_page_id = chapter_path_to_id(path);
        toc_fix.push_str(&format!(
            "<a href=\"#{}\">{}</a>", print_page_id, print_page_id
        ));
    }
    toc_fix.push_str("</div>");

    // 在 </body> 前插入
    if let Some(pos) = html.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + toc_fix.len() + 16);
        result.push_str(&html[..pos]);
        result.push_str(&toc_fix);
        result.push_str(&html[pos..]);
        result
    } else {
        // 没有 </body>，直接追加
        format!("{html}\n{toc_fix}")
    }
}

/// 将章节路径转换为 PDF 命名目标 ID
///
/// `path/to/chapter.md` → `path-to-chapter`
pub fn chapter_path_to_id(path: &str) -> String {
    let mut base = path.to_string();
    if base.ends_with(".md") {
        base.truncate(base.len() - 3);
    }
    base.replace(['/', '\\'], "-").to_ascii_lowercase()
}

// ─────────────────────────────────────────────────────────────────────────
// 2. JS 注入 — details 展开、MathJax 挂钩、内容加载哨兵
// ─────────────────────────────────────────────────────────────────────────

/// 注入 JavaScript 脚本以：
/// - 展开所有 `<details>` 元素（确保内容可见）
/// - 挂钩 MathJax 完成事件
/// - 添加内容加载哨兵元素（供 CDP 等待）
///
/// 返回 (处理后的 HTML, 是否注入了哨兵)
fn inject_js(html: &str) -> (String, bool) {
    let script = r#"
        <!-- mdbook-pdf: 自定义 JS 脚本 -->
        <script type='text/javascript'>
            let markAllContentHasLoadedForPrinting = () =>
                window.setTimeout(
                    () => {
                        let p = document.createElement('div');
                        p.setAttribute('id', 'content-has-all-loaded-for-mdbook-pdf-generation');
                        document.body.appendChild(p);
                    }, 100
                );

            window.addEventListener('load', () => {
                // 展开所有 <details> 元素以供打印
                let details = document.getElementsByTagName('details');
                for (let i of details)
                    i.open = true;

                try {
                    MathJax.Hub.Register.StartupHook('End', markAllContentHasLoadedForPrinting);
                } catch (e) {
                    markAllContentHasLoadedForPrinting();
                }
            });
        </script>
    "#;

    if let Some(pos) = html.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + script.len() + 16);
        result.push_str(&html[..pos]);
        result.push_str(script);
        result.push_str(&html[pos..]);
        (result, true)
    } else {
        // 没有 </body>，直接追加
        (format!("{html}\n{script}"), true)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 3. 链接修正
// ─────────────────────────────────────────────────────────────────────────

/// 将 HTML 中的相对路径链接修正为绝对 URL。
///
/// 仅当 `base_url` 非空时执行。修正规则：
/// - 仅处理 `<a href="...">` 中的 `href` 属性
/// - 跳过已为绝对链接（http://、https://、mailto: 等）、锚点（#）、协议相对（//）的链接
/// - 跳过以 `/` 开头的绝对路径链接（由静态站点自行处理）
fn fix_links(html: &str, base_url: &str) -> String {
    if base_url.is_empty() {
        return html.to_string();
    }

    let base_url = base_url.trim_end_matches('/');

    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href]").expect("a[href] selector is valid");

    // 收集所有需要修改的 (原 href, 新 href)
    let mut replacements: Vec<(String, String)> = Vec::new();

    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            if let Some(fixed) = fix_single_link(href, base_url) {
                replacements.push((href.to_string(), fixed));
            }
        }
    }

    if replacements.is_empty() {
        return html.to_string();
    }

    // 在原始 HTML 中替换 href 属性值
    let mut result = html.to_string();
    for (old_href, new_href) in &replacements {
        let search_pattern = format!("href=\"{}\"", old_href);
        let replace_pattern = format!("href=\"{}\"", new_href);
        result = result.replace(&search_pattern, &replace_pattern);
    }

    result
}

/// 判断并修复单个链接。
///
/// 返回 `Some(新URL)` 表示需要修复，`None` 表示保持原样。
fn fix_single_link(href: &str, base_url: &str) -> Option<String> {
    let href = href.trim();

    // 跳过绝对链接
    if href.starts_with("http://")
        || href.starts_with("https://")
        || href.starts_with("mailto:")
        || href.starts_with("tel:")
    {
        return None;
    }

    // 跳过锚点、JavaScript、协议相对
    if href.starts_with('#') || href.starts_with("javascript:") || href.starts_with("//") {
        return None;
    }

    // 跳过以 / 开头的绝对路径（静态站点处理）
    if href.starts_with('/') {
        return None;
    }

    // 相对路径：拼接 base_url
    let fixed = format!("{}/{}", base_url, href);
    Some(fixed)
}

// ─────────────────────────────────────────────────────────────────────────
// 4. 分页保护 CSS 注入
// ─────────────────────────────────────────────────────────────────────────

/// 注入增强的 `@media print` 分页控制 CSS 到 HTML 的 `</head>` 前。
///
/// 仅注入分页控制 CSS（不断页、不孤行），不做任何页眉/页脚注入。
/// 页眉/页脚完全由 CDP `displayHeaderFooter` 原生控制。
fn inject_print_css(html: &str) -> String {
    let print_css = r#"
@media print {
  body {
    -webkit-print-color-adjust: exact;
    print-color-adjust: exact;
  }
  h1, h2, h3, h4, h5, h6 { page-break-after: avoid; }
  h1 { page-break-before: always; }
  h2, h3 { page-break-before: avoid; }
  pre, code, table, figure, img, svg {
    page-break-inside: avoid;
  }
  .mermaid, .echarts {
    page-break-inside: avoid;
  }
  p, li { widows: 2; orphans: 2; }
}
"#;

    let style_tag = format!("<style>\n{print_css}</style>\n");

    if let Some(pos) = html.find("</head>") {
        let mut result = String::with_capacity(html.len() + print_css.len() + 16);
        result.push_str(&html[..pos]);
        result.push_str(&style_tag);
        result.push_str(&html[pos..]);
        result
    } else {
        format!("{style_tag}\n{html}")
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5. 字体 CSS 注入 — 解决 PDF 中特殊符号/中文显示为方框乱码的问题
// ─────────────────────────────────────────────────────────────────────────

/// 注入 `@font-face` 和 `font-family` CSS 规则，指定常见系统 CJK 字体。
///
/// Chrome 在生成 PDF 时，如果页面中的字符没有对应的系统字体，会显示为方框（tofu）。
/// 此函数注入 CSS 告诉 Chrome 优先使用系统中已安装的 CJK 字体。
///
/// 支持的字体（按优先级）：
/// - Linux: Noto Sans CJK SC, WenQuanYi Micro Hei
/// - macOS: PingFang SC, Hiragino Sans GB
/// - Windows: Microsoft YaHei, SimSun
fn inject_font_css(html: &str) -> String {
    let font_css = r#"
/* mdbook-pdf: CJK 字体回退 — 避免方框乱码 */
body {
  font-family:
    'Noto Sans CJK SC', 'Noto Sans SC', 'Source Han Sans SC',
    'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei',
    'WenQuanYi Micro Hei',
    serif, sans-serif;
}
code, pre, kbd {
  font-family:
    'Noto Sans CJK SC', 'Noto Sans SC', 'Source Han Sans SC',
    'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei',
    'WenQuanYi Micro Hei',
    'Fira Code', 'Cascadia Code', 'Source Code Pro',
    'Courier New', monospace;
}
"#;

    let style_tag = format!("<style>\n{font_css}</style>\n");

    if let Some(pos) = html.find("</head>") {
        let mut result = String::with_capacity(html.len() + font_css.len() + 16);
        result.push_str(&html[..pos]);
        result.push_str(&style_tag);
        result.push_str(&html[pos..]);
        result
    } else {
        format!("{style_tag}\n{html}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ToC 修复测试 ──

    #[test]
    fn test_chapter_path_to_id() {
        assert_eq!(chapter_path_to_id("intro.md"), "intro");
        assert_eq!(chapter_path_to_id("chapter/01-setup.md"), "chapter-01-setup");
    }

    #[test]
    fn test_inject_toc_fix_empty_paths() {
        let html = "<html><body><p>content</p></body></html>";
        let result = inject_toc_fix(html, &[]);
        assert_eq!(result, html);
    }

    #[test]
    fn test_inject_toc_fix_with_paths() {
        let html = "<html><body><p>content</p></body></html>";
        let paths = vec!["intro.md".to_string(), "chapter/setup.md".to_string()];
        let result = inject_toc_fix(html, &paths);
        assert!(result.contains(r##"<a href="#intro">intro</a>"##));
        assert!(result.contains(r##"<a href="#chapter-setup">chapter-setup</a>"##));
        assert!(result.contains(r#"<div style="display: none">"#));
        assert!(result.contains("</body>"));
        // ToC div should be before </body>
        let body_pos = result.rfind("</body>").unwrap();
        let toc_pos = result.find("intro").unwrap();
        assert!(toc_pos < body_pos);
    }

    // ── JS 注入测试 ──

    #[test]
    fn test_inject_js_has_script() {
        let html = "<html><body><p>test</p></body></html>";
        let (result, has_sentinel) = inject_js(html);
        assert!(has_sentinel);
        assert!(result.contains("content-has-all-loaded-for-mdbook-pdf-generation"));
        assert!(result.contains("MathJax.Hub"));
        assert!(result.contains("details"));
    }

    #[test]
    fn test_inject_js_no_body() {
        let html = "<html><p>test</p></html>";
        let (result, has_sentinel) = inject_js(html);
        assert!(has_sentinel);
        assert!(result.contains("content-has-all-loaded-for-mdbook-pdf-generation"));
    }

    // ── 链接修正测试 ──

    #[test]
    fn test_fix_links_empty_base() {
        let html = r#"<a href="page.html">link</a>"#;
        let result = fix_links(html, "");
        assert_eq!(result, html);
    }

    #[test]
    fn test_fix_relative_link() {
        let html = r#"<a href="page.html">link</a>"#;
        let result = fix_links(html, "https://example.com/book");
        assert!(result.contains(r#"href="https://example.com/book/page.html""#));
    }

    #[test]
    fn test_fix_absolute_link_skipped() {
        let html = r#"<a href="https://other.com/page.html">link</a>"#;
        let result = fix_links(html, "https://example.com/book");
        assert_eq!(result, html);
    }

    #[test]
    fn test_fix_anchor_skipped() {
        let html = "<a href=\"#section\">link</a>";
        let result = fix_links(html, "https://example.com/book");
        assert_eq!(result, html);
    }

    #[test]
    fn test_fix_root_path_skipped() {
        let html = r#"<a href="/absolute/page.html">link</a>"#;
        let result = fix_links(html, "https://example.com/book");
        assert_eq!(result, html);
    }

    #[test]
    fn test_fix_multiple_links() {
        let html = r#"<a href="a.html">a</a> <a href="https://ext.com/b.html">b</a> <a href="c.html">c</a>"#;
        let result = fix_links(html, "https://base.com");
        assert!(result.contains(r#"href="https://base.com/a.html""#));
        assert!(result.contains(r#"href="https://ext.com/b.html""#));
        assert!(result.contains(r#"href="https://base.com/c.html""#));
    }

    // ── 分页 CSS 注入测试 ──

    #[test]
    fn test_inject_print_css_has_head() {
        let html = "<html><head><title>T</title></head><body><p>text</p></body></html>";
        let result = inject_print_css(html);
        assert!(result.contains("page-break-inside: avoid"));
        assert!(result.contains("</head>"));
        let head_pos = result.find("</head>").unwrap();
        let css_pos = result.find("page-break-inside").unwrap();
        assert!(css_pos < head_pos, "CSS should be before </head>");
    }

    #[test]
    fn test_inject_print_css_no_head() {
        let html = "<body><p>no head</p></body>";
        let result = inject_print_css(html);
        assert!(result.starts_with("<style>"));
        assert!(result.contains("page-break-inside: avoid"));
    }

    // ── 完整流水线测试 ──

    #[test]
    fn test_preprocess_html_pipeline() {
        let html = r#"<html><head><title>T</title></head><body><a href="page.html">link</a><p>text</p></body></html>"#;
        let mut cfg = PdfOptions::default();
        cfg.static_site_url = "https://example.com/book".to_string();
        cfg.generate_document_outline = true;
        let paths = vec!["intro.md".to_string()];
        let result = preprocess_html(html, &cfg, &paths);
        let r = &result.html;
        assert!(r.contains(r#"href="https://example.com/book/page.html""#));
        assert!(r.contains("page-break-inside: avoid"));
        assert!(r.contains("content-has-all-loaded-for-mdbook-pdf-generation"));
        assert!(r.contains("href=\"#intro\""));
        assert!(result.has_content_sentinel);
    }
}
