//! 插件通用的工具函数

use std::io::IsTerminal;

/// 标准的 mdbook 预处理器入口：从 stdin 读取，处理，写入 stdout
///
/// 统一计时并在处理完成后输出同格式进度条（全英文）：
/// `[====] 100% - <preprocessor>: done (<elapsed>)`，用于分析各
/// preprocessor 的性能瓶颈。
pub fn run_preprocessor<P: mdbook_preprocessor::Preprocessor>(
    pre: &P,
) -> anyhow::Result<()> {
    let (ctx, book) = mdbook_preprocessor::parse_input(std::io::stdin())?;

    let book_version = semver::Version::parse(&ctx.mdbook_version)?;
    let version_req = semver::VersionReq::parse(mdbook_preprocessor::MDBOOK_VERSION)?;
    if !version_req.matches(&book_version) {
        log::debug!(
            "{} was built against mdbook v{}, but running with v{}",
            pre.name(),
            mdbook_preprocessor::MDBOOK_VERSION,
            ctx.mdbook_version,
        );
    }

    let processed = pre.run(&ctx, book)?;
    serde_json::to_writer(std::io::stdout(), &processed)?;
    // 总耗时由 print_progress 自动追加（累计计时），label 只写阶段名
    print_progress(1, 1, &format!("{}: done", pre.name()));
    Ok(())
}

/// 标准的 mdbook 渲染器入口：从 stdin 读取 RenderContext，处理
///
/// 统一计时并在处理完成后输出同格式进度条（全英文）：
/// `[====] 100% - <renderer>: done (<elapsed>)`。注意 PDF 渲染器内部
/// 已有分阶段进度条，此处的总耗时是补充信息。
pub fn run_renderer<R: mdbook_renderer::Renderer>(renderer: &R) -> anyhow::Result<()> {
    let ctx = mdbook_renderer::RenderContext::from_json(std::io::stdin())?;
    renderer.render(&ctx)?;
    // 总耗时由 print_progress 自动追加
    print_progress(1, 1, &format!("{}: done", renderer.name()));
    Ok(())
}

/// 计算从章节路径到 images 目录的相对路径前缀
///
/// 例如: "index.md" → "./images/", "test/7.md" → "../images/"
pub fn relative_svg_prefix(chapter_path: &std::path::Path) -> String {
    let depth = chapter_path.parent().map(|p| p.components().count()).unwrap_or(0);
    if depth == 0 {
        "./images/".to_string()
    } else {
        let parents: Vec<&str> = std::iter::repeat("..").take(depth).collect();
        format!("{}/images/", parents.join("/"))
    }
}

/// 将引擎生成的 SVG 字符串转换为适合直接内联进 HTML 的形式。
///
/// 做四件事：
/// 1. 剥离开头的 `<!-- Source: ... -->` 注释（写入文件时附加的调试信息，内联时不需要）；
/// 2. 删除所有空行——预处理器的输出会再次经过 pulldown-cmark 解析，
///    多行 HTML 块遇空行即截断，内联 SVG 必须保持连续；
/// 3. 在根 `<svg>` 元素上注入内联 `style`：
///    - `height:auto;` 配合 CSS 的 `width:100%` 保持宽高比；
///    - `font-family:serif;` 防御性字体隔离——阻断 mdbook 主题全局
///      `body { font-family: ... }` 对内联 SVG 的继承。
///      当前 TikZ(hayro-svg)/Typst(typst-svg) 都把文本字形转为 `<path>` 轮廓，
///      渲染结果天然与字体无关，此项为未来可能出现 `<text>` 内容时的兜底。
/// 4. 在 svg 前插入一段 `<style>`：
///    - **屏幕显示**：`width:100%` **强制放大到容器宽度**——mdbook 中 SVG 以
///      原始 pt 尺寸（如 185px）内联时，`max-width:100%` 只会缩小不会放大，
///      细线（0.2–0.4pt）与 10pt 文字会几乎不可见，视觉上"图空/字符乱"；
///    - **打印**：`width:auto;max-width:100%` 保持原始尺寸（打印分辨率高，
///      细线仍清晰）并防止大图溢出页面。
pub fn svg_to_inline(svg: &str) -> String {
    let svg = strip_xml_comment_header(svg);
    if !svg.contains("<svg") {
        return svg.to_string();
    }
    let svg = svg
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let svg = inject_svg_root_style(&svg);
    format!("{DIAGRAM_SVG_CSS}{svg}")
}

/// 内联 SVG 的响应式样式：屏幕放大到容器宽、打印保持原始尺寸防溢出。
/// 注入在 svg 之前（同一 HTML 块内，pulldown-cmark / ren-pdf 均保留）。
///
/// 末尾的 `.diagram-inline-svg text` 规则是文本层的兜底隔离：TikZ/Typst 的 SVG
/// 视觉层是 path 轮廓，同时叠加一层透明 `<text>`（可选中/可搜索）。内联时该
/// text 层会继承页面字体并可能被页面 CSS 覆盖 fill 而显现，与轮廓叠加成
/// "字体重叠混乱"；`<img>` 方式加载的 SVG 是独立文档、只受 UA 默认样式影响，
/// 不存在此问题。此规则 + 生成端注入的内联 style（`SVG_TEXT_LAYER_STYLE`）
/// 把 text 层锁定回与独立文档一致的渲染（透明 + serif）。
const DIAGRAM_SVG_CSS: &str = "<style>.diagram-inline-svg{max-width:100%;height:auto}@media screen{.diagram-inline-svg{width:100%}}@media print{.diagram-inline-svg{width:auto;max-width:100%}}.diagram-inline-svg text{fill:transparent!important;font-family:serif!important}</style>";

/// 文本层 `<text>` 的内联样式：锁定 `fill:transparent` + `font-family:serif`，
/// 与 `<img>` 独立文档加载时的 UA 默认渲染一致。内联 style + `!important` 的
/// 优先级高于任何页面选择器规则（除同样 `!important` 的内联 style），确保内联
/// SVG 的文本层字形不会以页面字体显现、与视觉层 path 轮廓叠加。
/// 仅在文本层生成端（typst/engine.rs、tikz/text_device.rs）使用。
pub(crate) const SVG_TEXT_LAYER_STYLE: &str =
    "style=\"fill:transparent!important;font-family:serif!important\"";

/// 剥离开头的 XML 注释（`<!-- ... -->`），返回第一个真实元素。
fn strip_xml_comment_header(svg: &str) -> &str {
    let s = svg.trim_start();
    if let Some(rest) = s.strip_prefix("<!--") {
        if let Some(end) = rest.find("-->") {
            return rest[end + 3..].trim_start();
        }
    }
    s
}

/// 注入到 `<svg>` 根元素的内联样式
const SVG_INLINE_STYLE: &str = "height:auto;font-family:serif;";

/// 在根 `<svg ...>` 标签上注入内联 style（引号感知地定位标签结束，
/// 避免属性值内的 `>` 被误判）。
fn inject_svg_root_style(svg: &str) -> String {
    let Some(open) = svg.find("<svg") else {
        return svg.to_string();
    };

    // 从 `<svg` 之后开始扫描到标签结束的 `>`（跳过引号内的字符）
    let mut end = open + 4;
    let bytes = svg.as_bytes();
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
    if end >= svg.len() {
        return svg.to_string();
    }

    let head = &svg[open..end]; // 含 <svg ...（不含 '>'）
    let rest = &svg[end..]; // 以 '>' 开头

    // 根元素已有 style 属性时，把我们的声明追加进其值里；否则新增 style 属性。
    // 引号类型按属性实际使用（双引号或单引号）定位值边界，避免误伤后续属性。
    let new_head = if let Some((style_pos, quote)) = find_attr(head, "style") {
        let after_name = style_pos + "style".len();
        match head[after_name..].find('=') {
            Some(eq_rel) => {
                let eq = after_name + eq_rel;
                let value_start = head[eq + 1..]
                    .find(quote)
                    .map(|p| eq + 1 + p + 1);
                match value_start {
                    Some(vs) => {
                        if let Some(rel_end) = head[vs..].find(quote) {
                            let ve = vs + rel_end;
                            // 在闭合引号前追加（保证以分号结尾）
                            let mut merged = head[..ve].to_string();
                            if !merged.ends_with(';') {
                                merged.push(';');
                            }
                            merged.push_str(SVG_INLINE_STYLE);
                            merged.push_str(&head[ve..]);
                            merged
                        } else {
                            head.to_string()
                        }
                    }
                    None => head.to_string(),
                }
            }
            None => head.to_string(),
        }
    } else {
        format!("{} style=\"{}\"", head, SVG_INLINE_STYLE)
    };
    // 统一追加响应式样式 class（已有 class 时不重复）
    let new_head = if find_attr(&new_head, "class").is_some() {
        new_head
    } else {
        format!("{} class=\"diagram-inline-svg\"", new_head)
    };

    format!("{}{}", new_head, rest)
}

/// 在标签内查找某个属性（如 `style`），返回 (属性起始下标, 属性值引号字符)。
/// 粗略但够用：只匹配 ` name` / `name=` 形式，忽略属性值内部。
fn find_attr(tag: &str, name: &str) -> Option<(usize, char)> {
    let mut search_from = 0;
    while let Some(rel) = tag[search_from..].find(name) {
        let pos = search_from + rel;
        // 确认该处确实是属性名：前一个字符是空白或标签开头
        let prev_ok = pos == 0 || tag.as_bytes()[pos - 1].is_ascii_whitespace();
        // 后一个字符是 '='（带可选空白）
        let after = pos + name.len();
        let eq_rel = tag[after..].find('=');
        let next_ok = eq_rel
            .map(|r| tag[after..][..r].trim().is_empty())
            .unwrap_or(false);
        if prev_ok && next_ok {
            let eq = after + eq_rel.unwrap();
            let val = tag[eq + 1..].trim_start();
            let quote = val
                .chars()
                .next()
                .filter(|c| *c == '"' || *c == '\'')
                .unwrap_or('"');
            return Some((pos, quote));
        }
        search_from = after;
    }
    None
}

/// XML 转义（用于把 Unicode 文本安全嵌入 SVG/HTML）
///
/// 同时把 **非法 XML 字符**（如 PDF 未映射字形占位符 `U+FFFF`、控制字符）
/// 替换为 `U+FFFD`（REPLACEMENT CHARACTER）——替换而非删除，保证字符数与
/// 坐标列表一一对应（text 层逐字符 x/y 定位），不破坏选中/搜索对齐。
pub(crate) fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if is_valid_xml_char(c) => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

/// XML 1.0 允许的字符：`#x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD] |
/// [#x10000-#x10FFFF]`。`U+FFFF`（以及 `U+FFFE`）不属于合法范围。
fn is_valid_xml_char(c: char) -> bool {
    matches!(c as u32,
        0x9 | 0xA | 0xD
        | 0x20..=0xD7FF
        | 0xE000..=0xFFFD
        | 0x10000..=0x10FFFF)
}


/// 独立输出进度条到 stderr，格式: " \x1b[32m INFO\x1b[0m [====>---]  12% - label (3.2s)"
///
/// 在终端中 INFO 显示为绿色，与 env_logger 的 info 级别颜色一致。
/// 输出到文件/管道时自动降级为纯文本 " INFO"。
/// 不依赖 log::info，避免时间戳和模块名前缀。
/// 自动记录首次调用时间，每次显示累计耗时。
/// - `current`: 当前进度序号（从 1 开始）
/// - `total`: 总步骤数
/// - `label`: 英文步骤描述
pub fn print_progress(current: u8, total: u8, label: &str) {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    let elapsed = start.elapsed();

    let pct = (current as f64) / (total as f64);
    let width: usize = 20;
    let filled = (pct * width as f64).round() as usize;
    let filled = filled.min(width);
    let pct_int = (pct * 100.0).round() as u8;

    let bar = if filled == 0 {
        format!("[>{}]", "-".repeat(width - 1))
    } else if filled >= width {
        format!("[{}]", "=".repeat(width))
    } else {
        format!(
            "[{}{}{}]",
            "=".repeat(filled - 1),
            ">",
            "-".repeat(width - filled)
        )
    };

    let elapsed_str = format_elapsed(elapsed);
    let info_prefix = if std::io::stderr().is_terminal() {
        "\x1b[32m INFO\x1b[0m"  // green
    } else {
        " INFO"
    };
    eprintln!("{} {} {:3}% - {} ({})", info_prefix, bar, pct_int, label, elapsed_str);
}

/// 格式化持续时间，如 "0.1s", "12.3s", "1m 23s", "2h 5m"
pub(crate) fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{:.1}s", secs)
    } else if secs < 3600.0 {
        let m = (secs / 60.0) as u64;
        let s = (secs % 60.0) as u64;
        format!("{}m {}s", m, s)
    } else {
        let h = (secs / 3600.0) as u64;
        let m = ((secs % 3600.0) / 60.0) as u64;
        format!("{}h {}m", h, m)
    }
}

/// 独立输出状态信息到 stderr，格式: " \x1b[32m INFO\x1b[0m <message>"
///
/// 与 print_progress 风格一致，终端中 INFO 显示为绿色。
/// 不受 RUST_LOG 级别影响，始终输出。
/// 输出到文件/管道时自动降级为纯文本 " INFO"。
pub fn print_status(msg: &str) {
    let info_prefix = if std::io::stderr().is_terminal() {
        "\x1b[32m INFO\x1b[0m"
    } else {
        " INFO"
    };
    eprintln!("{} {}", info_prefix, msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inline_injects_style() {
        let svg = r#"<svg viewBox="0 0 10 10" xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#;
        let out = svg_to_inline(svg);
        assert!(
            out.contains(r#"style="height:auto;font-family:serif;""#),
            "缺少注入的 style: {}",
            out
        );
        assert!(out.contains("class=\"diagram-inline-svg\""), "缺少 class: {}", out);
        assert!(out.starts_with("<style>"), "缺少响应式 <style>: {}", out);
        assert!(out.contains("@media print"), "缺少打印样式: {}", out);
        assert!(out.contains(r#"<svg viewBox="0 0 10 10""#), "根元素必须保留: {}", out);
        assert!(out.ends_with("</svg>"), "svg 必须完整闭合: {}", out);
    }

    #[test]
    fn test_inline_merges_existing_style() {
        let svg = r#"<svg style="background-color:red;" viewBox="0 0 1 1"><path/></svg>"#;
        let out = svg_to_inline(svg);
        assert!(
            out.contains(r#"style="background-color:red;height:auto;font-family:serif;""#),
            "style 合并失败: {}",
            out
        );
        assert!(out.contains("class=\"diagram-inline-svg\""), "缺少 class: {}", out);
    }

    #[test]
    fn test_inline_single_quote_style() {
        let svg = r#"<svg style='background-color:blue' viewBox="0 0 1 1"><path/></svg>"#;
        let out = svg_to_inline(svg);
        assert!(
            out.contains(r#"style='background-color:blue;height:auto;font-family:serif;'"#),
            "单引号 style 合并失败: {}",
            out
        );
        // 单引号场景下 viewBox 属性必须完好
        assert!(out.contains(r#"viewBox="0 0 1 1""#), "后续属性被破坏: {}", out);
    }

    #[test]
    fn test_inline_strips_source_comment_and_blank_lines() {
        let svg = "<!-- Source: test/7.latex_pictures.md -->\n\n<svg viewBox=\"0 0 1 1\">\n\n<path d=\"M0 0\"/>\n</svg>\n";
        let out = svg_to_inline(svg);
        assert!(!out.contains("Source:"), "注释应被剥离: {}", out);
        assert!(!out.contains("\n\n"), "空行应被压缩: {}", out);
        assert!(out.starts_with("<style>"), "应以 <style> 开头: {}", out);
        assert!(out.contains("<svg viewBox=\"0 0 1 1\""), "svg 应在 style 后: {}", out);
    }

    #[test]
    fn test_inline_no_svg_tag() {
        let out = svg_to_inline("not an svg");
        assert_eq!(out, "not an svg");
    }

    #[test]
    fn test_text_layer_isolation() {        // 文本层兜底规则必须存在：内联 SVG 的 <text> 锁定透明 + serif，
        // 防止页面 CSS 覆盖 fill 后字形与 path 轮廓叠加成"字体重叠混乱"
        assert!(
            DIAGRAM_SVG_CSS.contains(".diagram-inline-svg text{fill:transparent!important;font-family:serif!important}"),
            "缺少文本层隔离规则: {}",
            DIAGRAM_SVG_CSS
        );
        // 生成端注入的内联 style：fill:transparent + font-family:serif，均 !important
        assert_eq!(
            SVG_TEXT_LAYER_STYLE,
            "style=\"fill:transparent!important;font-family:serif!important\""
        );
    }

    #[test]
    fn test_inline_attribute_value_with_gt() {
        // 属性值（data URI）中包含 '>' 不应提前终止根标签扫描
        let svg = r#"<svg viewBox="0 0 1 1"><image href="data:image/png;base64,AAA>BBB"/></svg>"#;
        let out = svg_to_inline(svg);
        assert!(out.contains(r#"<svg viewBox="0 0 1 1" style="height:auto;font-family:serif;" class="diagram-inline-svg""#), "根标签注入失败: {}", out);
        assert!(out.contains("AAA>BBB"), "data URI 内容被破坏: {}", out);
    }

    #[test]
    fn test_escape_xml_replaces_illegal_chars() {
        // U+FFFF（PDF 未映射字形占位符）等非法 XML 字符必须被替换（保持长度）
        let input = format!("a{}b\u{7}c\u{FFFE}\u{FFFF}", '\u{0}');
        let out = escape_xml(&input);
        assert!(!out.contains('\u{FFFF}'), "U+FFFF 未被替换: {:?}", out);
        assert!(!out.contains('\u{FFFE}'), "U+FFFE 未被替换: {:?}", out);
        assert_eq!(out.chars().count(), input.chars().count(), "字符数必须不变（坐标对齐）");
        // 合法字符保留且顺序不变（\u{0} 和 \u{7} 被替换为 U+FFFD）
        assert_eq!(out.chars().collect::<Vec<_>>()[0], 'a');
        assert!(out.contains('b') && out.contains('c'));
        // 非法字符替换为 U+FFFD
        assert!(out.contains('\u{FFFD}'));
        // 常规转义不受影响
        assert_eq!(escape_xml("<a&b>\"'"), "&lt;a&amp;b&gt;&quot;&apos;");
        // 中文等合法 Unicode 保留
        assert_eq!(escape_xml("你好 ∇"), "你好 ∇");
    }
}
