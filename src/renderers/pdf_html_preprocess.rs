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

/// 删除阻塞型外部 CDN 脚本，保留图片和样式表
///
/// `<script>` 标签是同步阻塞资源，在 WSL 中加载极慢（数分钟），
/// 会阻塞 HTML 解析和 window.load。删除它们后页面加载不受外部 CDN 影响。
/// `<link>` 样式表和 `<img>` 图片是并行加载资源，保留以保证 PDF 内容和样式完整。
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
            if tag.starts_with("<script ") || tag.starts_with("<script>") {
                // 脚本是同步阻塞资源，必须删除
                result.push_str("<script>/* CDN removed for PDF */</script>");
            } else if tag.starts_with("<link") && tag.contains("stylesheet") {
                // 外部样式表：
                // - font-awesome（cdnjs 极慢）对 PDF 打印无意义，直接删除；
                // - katex.min.css **必须保留**（cdn.jsdelivr.net 快且 KaTeX 公式
                //   样式依赖它；转 media="print" 会导致 printToPDF 不加载 → 公式
                //   排版失效）；
                // - 其余转 media="print" —— 不阻塞 window.load，打印时加载。
                if tag.contains("font-awesome") {
                    result.push_str("<!-- CDN font-awesome removed for PDF -->");
                } else if tag.contains("katex") {
                    result.push_str(tag);
                } else if tag.contains("media=") {
                    result.push_str(tag);
                } else {
                    result.push_str(&tag.replacen("<link", "<link media=\"print\"", 1));
                }
            } else {
                // 图片等其他资源：保留以保证 PDF 内容完整
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

/// 删除内联 `<script type="module">` 中的 CDN import（如 latex.js 的
/// `import ... from "https://cdn.jsdelivr.net/..."`）。
///
/// module script 的顶层 import 是同步阻塞的——浏览器在 DOMContentLoaded 之前
/// 必须下载并执行它；CDN 慢/不可达时会让页面加载卡几十秒（PDF 渲染实测
/// 1m+）。PDF 渲染不依赖这些 CDN 模块，直接删除以消除阻塞。
pub fn remove_cdn_module_scripts(html: &str) -> String {
    let re = regex::Regex::new(
        r#"(?s)<script[^>]*type=["']module["'][^>]*>.*?from\s+["']https?://[^"']+["'].*?</script>"#,
    )
    .unwrap();
    re.replace_all(html, "<script>/* CDN module removed for PDF */</script>")
        .to_string()
}

/// 注入 JS 脚本:
/// - enable_emoji=true: 使用 @font-face 的 COLR 矢量 emoji 字体（可嵌入 PDF，体积小）
/// - enable_emoji=false: 跳过 Emoji 处理，系统字体回退。注意：**不节省渲染时间**
///   （PDF 耗时主要在 window.load 等资源加载，与 emoji 无关），且彩色 emoji
///   系统字体（如 Segoe UI Emoji）无法嵌入 PDF，会被位图化导致 PDF 体积暴涨
///   （实测 26MB → 96MB）。建议保持默认开启。
/// - 执行顺序：window.load → DOM操作 → Emoji处理 → 2帧RAF → 哨兵
pub fn inject_js(html: &str, enable_emoji: bool) -> String {
    // 1. Emoji 函数定义（必须放在最前面，在调用之前）
    let emoji_defs = if enable_emoji {
        r#"
// ── Emoji 处理：优化版 ──
// 使用二分查找代替 Map 构建（零初始化开销）
// 使用快速跳过优化（纯 ASCII 文本直接跳过）
var EMOJI_RANGES = [
    [0x00A9,0x00AE],
    [0x200D,0x200D],
    [0x203C,0x2049],
    [0x20E3,0x20E3],
    [0x2122,0x2139],
    [0x2194,0x2199],
    [0x21A9,0x21AA],
    [0x231A,0x231B],
    [0x2328,0x2328],
    [0x23CF,0x23CF],
    [0x23E9,0x23F3],
    [0x23F8,0x23FA],
    [0x24C2,0x24C2],
    [0x25AA,0x25AB],
    [0x25B6,0x25B6],
    [0x25C0,0x25C0],
    [0x25FB,0x25FE],
    [0x2600,0x27BF],
    [0x2934,0x2935],
    [0x2B05,0x2B07],
    [0x2B1B,0x2B1C],
    [0x2B50,0x2B50],
    [0x2B55,0x2B55],
    [0x3030,0x3030],
    [0x303D,0x303D],
    [0x3297,0x3297],
    [0x3299,0x3299],
    [0xFE00,0xFE0F],
    [0x1F000,0x1FFFF],
    [0x1F1E6,0x1F1FF],
];

function isEmoji(cp) {
    if (cp < 0x00A9 || cp > 0x1FFFF) return false;
    var lo = 0, hi = EMOJI_RANGES.length - 1;
    while (lo <= hi) {
        var mid = (lo + hi) >> 1;
        var r = EMOJI_RANGES[mid];
        if (cp < r[0]) hi = mid - 1;
        else if (cp > r[1]) lo = mid + 1;
        else return true;
    }
    return false;
}

// 快速检测：文本是否可能包含 emoji（使用 charCodeAt，比 codePointAt 快）
function hasEmoji(text) {
    for (var i = 0; i < text.length; i++) {
        var cp = text.charCodeAt(i);
        // 纯 ASCII 快速跳过（大部分技术文档正文）
        if (cp < 0x00A9) continue;
        // BMP 范围快速检查
        if (cp <= 0x00AE || (cp >= 0x200D && cp <= 0x2049) ||
            (cp >= 0x20E3 && cp <= 0x20E3) ||
            (cp >= 0x2122 && cp <= 0x2139) || (cp >= 0x2194 && cp <= 0x21AA) ||
            (cp >= 0x231A && cp <= 0x23F3) || (cp >= 0x23F8 && cp <= 0x23FA) ||
            (cp >= 0x24C2 && cp <= 0x24C2) || (cp >= 0x25AA && cp <= 0x25FE) ||
            (cp >= 0x2600 && cp <= 0x27BF) || (cp >= 0x2934 && cp <= 0x2935) ||
            (cp >= 0x2B05 && cp <= 0x2B07) || (cp >= 0x2B1B && cp <= 0x2B1C) ||
            (cp >= 0x2B50 && cp <= 0x2B50) || (cp >= 0x2B55 && cp <= 0x2B55) ||
            (cp >= 0x3030 && cp <= 0x3030) || (cp >= 0x303D && cp <= 0x303D) ||
            (cp >= 0x3297 && cp <= 0x3297) || (cp >= 0x3299 && cp <= 0x3299) ||
            (cp >= 0xFE00 && cp <= 0xFE0F)) {
            return true;
        }
        // 代理对检查（补充平面 emoji）
        if (cp >= 0xD800 && cp <= 0xDBFF && i + 1 < text.length) {
            var low = text.charCodeAt(i + 1);
            if (low >= 0xDC00 && low <= 0xDFFF) {
                var codePoint = ((cp - 0xD800) << 10) + (low - 0xDC00) + 0x10000;
                if (codePoint >= 0x1F000 && codePoint <= 0x1FFFF) return true;
            }
        }
    }
    return false;
}

// 计算 emoji 序列长度（处理 ZWJ、键帽、国旗等复合序列）
function emojiLen(text, i) {
    if (i >= text.length) return 0;
    var cp = text.codePointAt(i);
    if (cp === 0x0023 || cp === 0x002A || (cp >= 0x0030 && cp <= 0x0039)) {
        var j = i + 1;
        if (j < text.length && text.codePointAt(j) === 0xFE0F) j++;
        if (j < text.length && text.codePointAt(j) === 0x20E3) return j + 1 - i;
        return 0;
    }
    if (!isEmoji(cp)) return 0;
    var cl = cp > 0xFFFF ? 2 : 1;
    var total = cl;
    for (var j = i + cl; j < text.length;) {
        var nc = text.codePointAt(j);
        if (nc === 0x200D) {
            total += 1; j += 1;
            if (j >= text.length) break;
            var ec = text.codePointAt(j);
            if (!isEmoji(ec)) break;
            var el = ec > 0xFFFF ? 2 : 1;
            total += el; j += el;
        } else if (nc === 0xFE0F || nc === 0xFE0E) {
            total += 1; j += 1;
        } else if (nc === 0x20E3) {
            total += 1; j += 1;
        } else if (nc >= 0x1F1E6 && nc <= 0x1F1FF && j + 1 < text.length) {
            var nc2 = text.codePointAt(j + 1);
            if (nc2 >= 0x1F1E6 && nc2 <= 0x1F1FF) { total += 4; j += 2; }
            else break;
        } else break;
    }
    return total;
}

// Emoji 处理：快速跳过 + 同步替换
function processEmojiSync() {
    var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    var emojiNodes = [];
    while (walker.nextNode()) {
        // 快速预过滤：大部分文本节点不包含 emoji
        if (hasEmoji(walker.currentNode.textContent)) {
            emojiNodes.push(walker.currentNode);
        }
    }

    for (var n = 0; n < emojiNodes.length; n++) {
        var node = emojiNodes[n], text = node.textContent, frag = null;
        var pos = 0, last = 0;
        while (pos < text.length) {
            var sl = emojiLen(text, pos);
            if (sl > 0) {
                if (!frag) frag = document.createDocumentFragment();
                if (pos > last) frag.appendChild(document.createTextNode(text.slice(last, pos)));
                var sp = document.createElement('span');
                sp.className = 'emoji-render';
                sp.textContent = text.slice(pos, pos + sl);
                frag.appendChild(sp);
                pos += sl; last = pos;
            } else {
                pos += (text.codePointAt(pos) > 0xFFFF ? 2 : 1);
            }
        }
        if (frag) {
            if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
            node.parentNode.replaceChild(frag, node);
        }
    }
}
"#
    } else {
        ""
    };

    // 2. window.load 回调中的 Emoji 处理调用
    let emoji_call = if enable_emoji {
        r#"
    // 3. Emoji 结构化隔离（正则预过滤 + 同步替换）
    processEmojiSync();
    window.__mdbookPdfEmojiDone = true;
"#
    } else {
        r#"
    // 3. Emoji 处理已禁用，直接标记完成
    window.__mdbookPdfEmojiDone = true;
"#
    };

    // 脚本结构：函数定义在前，调用在后
    let script = format!(r#"<script type='text/javascript'>
// ── Emoji 函数定义（必须先于调用）──
{}
// ── 全局进度状态标记（供 Rust 侧轮询）──
window.__mdbookPdfDomReady = false;
window.__mdbookPdfPageLoaded = false;
window.__mdbookPdfFontsReady = false;
window.__mdbookPdfEmojiDone = false;
window.__mdbookPdfContentReady = false;
window.__pdfRenderLatch = window.__pdfRenderLatch || [];

// ── 阶段 1：DOM 就绪 ──
document.addEventListener('DOMContentLoaded', function () {{
    window.__mdbookPdfDomReady = true;
}});

// ── 阶段 2：window.load（等待所有资源，包括字体，加载完成）──
window.addEventListener('load', function () {{
    window.__mdbookPdfPageLoaded = true;
    window.__mdbookPdfFontsReady = true;

    // 1. DOM 操作：展开 details，移除页眉/页脚
    for (var d of document.getElementsByTagName('details')) d.open = true;
    var ph = document.getElementById('mdbook-print-header');
    var pf = document.getElementById('mdbook-print-footer');
    if (ph) ph.remove();
    if (pf) pf.remove();

    // 2. Emoji 处理（同步执行，正则预过滤极快）
    {}

    // 3. 等 2 帧 — 确保布局完成
    var frameCount = 2;
    function frame() {{
        if (--frameCount <= 0) {{
            window.__mdbookPdfContentReady = true;
            var p = document.createElement('div');
            p.setAttribute('id', 'content-has-all-loaded-for-mdbook-pdf-generation');
            document.body.appendChild(p);
        }} else {{
            requestAnimationFrame(frame);
        }}
    }}
    requestAnimationFrame(frame);
}});

// MathJax 异步加载兼容
(function () {{
    var mjTimer = setInterval(function () {{
        try {{
            if (window.MathJax && MathJax.Hub) {{
                clearInterval(mjTimer);
                MathJax.Hub.Register.StartupHook('End', function () {{
                    window.__pdfRenderLatch.push(Promise.resolve());
                }});
            }}
        }} catch (e) {{}}
    }}, 200);
    setTimeout(function () {{ clearInterval(mjTimer); }}, 60000);
}})();
</script>"#, emoji_defs, emoji_call);

    insert_before(html, "</body>", &script)
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

/// 为 TikZ/Typst 图形添加唯一 ID，供 PDF 后处理页面定位使用
///
/// 找到所有 `data-pdf-hash` 属性所在的 `<div>` 标签，直接添加
/// `id="tikz-grp-{n}"` 属性（不插入额外元素，不破坏 HTML 结构）。
/// Chrome printToPDF 会将这些 ID 转换为 PDF 命名目标。
pub fn inject_tikz_anchors(html: &str) -> String {
    let marker = "data-pdf-hash=\"";
    let mut result = String::with_capacity(html.len() + 512);
    let mut remaining = html;
    let mut count = 0;

    loop {
        match remaining.find(marker) {
            None => {
                result.push_str(remaining);
                break;
            }
            Some(pos) => {
                // 保留 marker 之前的内容
                result.push_str(&remaining[..pos]);
                // 在 data-pdf-hash 属性之前添加 id 属性
                result.push_str(&format!(r#"id="tikz-grp-{}" "#, count));
                result.push_str(marker);
                remaining = &remaining[pos + marker.len()..];
                count += 1;
            }
        }
    }

    if count > 0 {
        log::debug!("为 {} 个 TikZ/Typst 图形添加了 id 属性", count);
    }
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
    let mut result = String::with_capacity(html.len() * 2);
    result.push_str(html);

    // 1. 链接修正
    if !cfg.static_site_url.is_empty() {
        result = fix_links(&result, &cfg.static_site_url);
        result.reserve(result.len() / 2);
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

    // 5b. 删除内联 module script 的 CDN import（如 latex.js，阻塞 DOMContentLoaded）
    result = remove_cdn_module_scripts(&result);

    // 5c. SVG img → 内联 <svg>（PDF 渲染专用，避免 img 引用 SVG 的慢渲染）
    if let Some(root) = book_root {
        result = inline_svg_images(&result, root);
    }

    // 6. JS 注入（根据 enable_emoji_font 配置决定是否包含 Emoji 处理）
    result = inject_js(&result, cfg.enable_emoji_font);

    // 7. 替换 PDF 预览容器为静态链接（PDF 中交互式容器无法工作）
    result = replace_pdf_containers(&result);

    // 7b. 为 TikZ/Typst 图形注入命名锚点（方案B: 后处理页面定位）
    result = inject_tikz_anchors(&result);

    // 8. Emoji 字体——仅在启用时注入 @font-face
    //    注意：关闭不会省时（耗时在 window.load 资源加载），且系统彩色 emoji
    //    字体无法嵌入 PDF 会位图化导致体积暴涨（26MB → 96MB），建议保持默认开启
    if cfg.enable_emoji_font {
    if let Some(root) = book_root {
        let fonts_dir = root.join("theme").join("fonts");
        if fonts_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&fonts_dir) {
                let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                files.sort_by_key(|e| e.file_name());
                for entry in &files {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.ends_with(".woff2") && name_str.contains("emoji") {
                        let rel = format!("../../theme/fonts/{}", name_str);
                        let css = format!(
                            "<style>\
                            @font-face {{ \
                            font-family: 'Emoji PDF'; \
                            font-style: normal; font-display: swap; font-weight: 400; \
                            src: url('{}') format('woff2'); \
                            }} \
                            .emoji-render {{ \
                            font-family: 'Emoji PDF', 'Noto Color Emoji', \
                            'Apple Color Emoji', 'Segoe UI Emoji', sans-serif; \
                            }} \
                            </style>",
                            rel
                        );
                        if let Some(pos) = result.rfind("</head>") {
                            result.insert_str(pos, &css);
                        }
                        break;
                    }
                }
            }
        }
    }
    } // end of if cfg.enable_emoji_font

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
        let result = inject_js(html, false);
        assert!(result.contains("__pdfRenderLatch"));
        assert!(result.contains("content-has-all-loaded-for-mdbook-pdf-generation"));
        assert!(result.contains("<script"));
    }

    #[test]
    fn test_inject_js_with_emoji_enabled() {
        let html = "<html><body><p>hello</p></body></html>";
        let result = inject_js(html, true);
        assert!(result.contains("processEmojiSync"));
        assert!(result.contains("isEmoji"));
        assert!(result.contains("hasEmoji"));
        assert!(result.contains("EMOJI_RANGES"));
    }

    #[test]
    fn test_inject_js_with_emoji_disabled() {
        let html = "<html><body><p>hello</p></body></html>";
        let result = inject_js(html, false);
        assert!(!result.contains("processEmojiParallel"));
        assert!(!result.contains("isEmoji"));
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

    #[test]
    fn test_rewrite_svg_ids_prefix() {
        let svg = r##"<defs><path id="g0"/><clipPath id="c1"/></defs><use xlink:href="#g0"/><path clip-path="url(#c1)"/>"##;
        let out = rewrite_svg_ids(svg, "abc");
        assert!(out.contains(r##"id="abcg0""##), "defs id 未加前缀: {}", out);
        assert!(out.contains(r##"id="abcc1""##), "clipPath id 未加前缀: {}", out);
        assert!(out.contains(r##"xlink:href="#abcg0""##), "use 引用未加前缀: {}", out);
        assert!(out.contains(r##"clip-path="url(#abcc1)""##), "clip-path 引用未加前缀: {}", out);
        assert!(!out.contains(r##"#g0"##) && !out.contains(r##"#c1"##), "旧引用残留: {}", out);
    }

    #[test]
    fn test_extract_svg_element() {
        let svg = "<!-- Source: test.md -->\n<svg viewBox=\"0 0 1 1\"><path/></svg>\n";
        let out = extract_svg_element(svg).unwrap();
        assert!(out.starts_with("<svg") && out.ends_with("</svg>"), "提取失败: {}", out);
        assert!(!out.contains("Source"), "注释未剥离: {}", out);
        assert_eq!(extract_svg_element("no svg"), None);
    }

    #[test]
    fn test_inject_svg_width_style() {
        let svg = r#"<svg viewBox="0 0 1 1" xmlns="http://www.w3.org/2000/svg"><path/></svg>"#;
        let out = inject_svg_width_style(svg);
        assert!(out.contains("width:100%;height:auto;max-width:100%"), "宽度样式未注入: {}", out);
        let svg2 = r#"<svg viewBox="0 0 1 1" style="color:red"><path/></svg>"#;
        let out2 = inject_svg_width_style(svg2);
        assert!(out2.contains("color:red"), "原 style 被破坏: {}", out2);
        assert!(out2.contains("width:100%"), "宽度样式未注入: {}", out2);
    }

    #[test]
    fn test_remove_external_resources_css() {
        // katex CSS 必须保留（公式样式依赖），font-awesome 删除，其他外部 CSS 转 media="print"
        let html = r#"<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.17.0/dist/katex.min.css">
<link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/font-awesome/4.7.0/css/font-awesome.min.css">
<link rel="stylesheet" href="https://example.com/other.css">
<script async src="https://example.com/lib.js"></script>
<link rel="stylesheet" href="local.css">"#;
        let out = remove_external_resources(html);
        // katex link 原样保留（不转 media="print"）
        assert!(out.contains(r#"<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.17.0/dist/katex.min.css">"#), "katex CSS 被改动: {}", out);
        assert!(!out.contains("font-awesome.min.css"), "font-awesome 未删除: {}", out);
        assert!(out.contains(r#"<link media="print" rel="stylesheet" href="https://example.com/other.css">"#), "其他外部 CSS 未转 media=print: {}", out);
        assert!(out.contains("CDN removed for PDF"), "外部 script 未删除: {}", out);
        assert!(out.contains("href=\"local.css\""), "本地 CSS 被误删: {}", out);
    }
}

/// 将 `<img src="...images/{name}.svg">` 内联为 `<svg>`（PDF 渲染专用）。
///
/// headless/无 GPU 环境下，img 引用 SVG 的渲染远慢于内联 SVG（实测 53s →
/// 29s，18 张图）。内联后每张 SVG 的 id 加文件名前缀，避免跨 SVG 的
/// `<use>`/`clip-path` 引用冲突（每页 defs 的 g0/c0 等重复）。
fn inline_svg_images(html: &str, book_root: &Path) -> String {
    let re = regex::Regex::new(r#"<img[^>]*src="[^"]*/([^"/]+\.svg)"[^>]*>"#).unwrap();
    let mut result = String::with_capacity(html.len());
    let mut last = 0;

    for cap in re.captures_iter(html) {
        let m = cap.get(0).unwrap();
        result.push_str(&html[last..m.start()]);

        let fname = &cap[1];
        let src_path = book_root.join("src").join("images").join(fname);
        if let Ok(svg) = std::fs::read_to_string(&src_path) {
            if let Some(svg) = extract_svg_element(&svg) {
                let prefix = fname.trim_end_matches(".svg");
                let svg = rewrite_svg_ids(&svg, prefix);
                let svg = inject_svg_width_style(&svg);
                result.push_str(&format!(
                    r#"<div style="text-align:center;margin:8px 0;">{}</div>"#,
                    svg
                ));
                last = m.end();
                continue;
            }
        }
        result.push_str(&html[m.start()..m.end()]);
        last = m.end();
    }
    result.push_str(&html[last..]);
    result
}

/// 从 SVG 文件中提取 `<svg>...</svg>` 元素（去掉 XML 声明与 Source 注释）。
fn extract_svg_element(svg: &str) -> Option<String> {
    let start = svg.find("<svg")?;
    let end = svg.rfind("</svg>")? + "</svg>".len();
    Some(svg[start..end].to_string())
}

/// 把 SVG 内所有「字母+数字」id 定义与引用加上前缀，消除多 SVG 内联冲突。
fn rewrite_svg_ids(svg: &str, prefix: &str) -> String {
    let re_defs = regex::Regex::new(r##"\bid="([a-z])(\d+)""##).unwrap();
    let re_refs = regex::Regex::new(r##"(url\(#|href="#)([a-z])(\d+)"##).unwrap();
    let s1 = re_defs.replace_all(svg, |caps: &regex::Captures| {
        format!("id=\"{}{}{}\"", prefix, &caps[1], &caps[2])
    });
    re_refs
        .replace_all(&s1, |caps: &regex::Captures| {
            format!("{}{}{}{}", &caps[1], prefix, &caps[2], &caps[3])
        })
        .into_owned()
}

/// 在根 `<svg>` 标签注入响应式宽度样式（合并已有 style）。
fn inject_svg_width_style(svg: &str) -> String {
    let style = "width:100%;height:auto;max-width:100%";
    if let Some(open) = svg.find("<svg") {
        let bytes = svg.as_bytes();
        let mut end = open + 4;
        let mut in_quote: Option<u8> = None;
        while end < svg.len() {
            let c = bytes[end];
            match in_quote {
                Some(q) => {
                    if c == q {
                        in_quote = None;
                    }
                }
                None => {
                    if c == b'"' || c == b'\'' {
                        in_quote = Some(c);
                    } else if c == b'>' {
                        break;
                    }
                }
            }
            end += 1;
        }
        if end < svg.len() {
            let tag = &svg[open..end];
            let (new_tag, tail) = if tag.ends_with('/') {
                (format!("{}{}", &tag[..tag.len() - 1], style), "/>")
            } else {
                (format!("{} {}", tag, style), "")
            };
            return format!("{}{}{}{}", &svg[..open], new_tag, tail, &svg[end..]);
        }
    }
    svg.to_string()
}
