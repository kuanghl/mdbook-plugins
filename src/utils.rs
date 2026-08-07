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

/// 图表容器「源码 ↔ 图片」切换按钮的样式（随容器内联注入，不依赖主题全局 CSS）。
///
/// - 容器 `position:relative` + `text-align:center`：**居中**渲染的图（内联 SVG /
///   `<img>`）——解决内联 SVG 左对齐不居中的问题；
/// - 按钮定位**左上角**，**平时隐藏**（`opacity:0;visibility:hidden`，不占交互），
///   鼠标**悬停到图片/容器**时才显现（仿 mdbook 代码复制按钮：平时隐藏、悬停代码
///   块显示图标）；用 `:focus-visible` 而非 `:focus`，避免点击后焦点残留导致一直显示；
/// - 悬停到按钮上会**悬浮出文字提示气泡**（`::after content:attr(data-tooltip)`，
///   仿 mdbook 复制按钮的 "Copy to clipboard" 提示）；
/// - 按钮放左上角，与 mdbook 右上角的代码复制按钮不重叠；
/// - 源码区用 `hidden` 属性控制显隐（不写死 `display`，避免覆盖 `hidden`）。
const DIAGRAM_TOGGLE_CSS: &str = "\
.diagram-box{position:relative;text-align:center;margin:0.6em auto;}\
.diagram-box .diagram-toggle-btn{position:absolute;top:8px;left:8px;z-index:6;\
opacity:0;visibility:hidden;transform:translateY(-3px);\
transition:opacity .15s ease,visibility .15s ease,transform .15s ease;\
padding:0;line-height:0;cursor:pointer;font-family:inherit;\
border:1px solid rgba(127,127,127,.4);border-radius:4px;\
width:26px;height:26px;display:flex;align-items:center;justify-content:center;\
background:rgba(255,255,255,.9);color:#444;box-shadow:0 1px 2px rgba(0,0,0,.08);}\
.diagram-box .diagram-toggle-btn svg{display:block;width:16px;height:16px;}\
.diagram-box:hover .diagram-toggle-btn,.diagram-box .diagram-toggle-btn:focus-visible{\
opacity:1;visibility:visible;transform:translateY(0);}\
.diagram-box .diagram-toggle-btn::after{content:attr(data-tooltip);\
position:absolute;bottom:calc(100% + 8px);left:50%;transform:translateX(-50%);\
white-space:nowrap;background:#333;color:#fff;padding:3px 8px;border-radius:4px;\
font-size:12px;line-height:1.4;opacity:0;visibility:hidden;\
transition:opacity .15s ease,visibility .15s ease;pointer-events:none;}\
.diagram-box .diagram-toggle-btn:hover::after,.diagram-box .diagram-toggle-btn:focus-visible::after{\
opacity:1;visibility:visible;}\
.diagram-box .diagram-toggle-source{text-align:left;max-height:420px;overflow:auto;\
margin:0;background:#fff;border:1px solid rgba(127,127,127,.12);border-radius:6px;\
padding:10px 12px;box-shadow:0 1px 3px rgba(0,0,0,.06);box-sizing:border-box;}\
.diagram-box .diagram-toggle-source code{white-space:pre-wrap;font-size:13px;}\
";

/// 「眼睛」图标（Feather icon, MIT）：查看源码用，无文字。
const DIAGRAM_TOGGLE_EYE_SVG: &str = "<svg viewBox=\"0 0 24 24\" fill=\"none\" \
stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" \
stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M1 12s4-8 11-8 11 8 11 \
8-4 8-11 8-11-8-11-8z\"/><circle cx=\"12\" cy=\"12\" r=\"3\"/></svg>";

/// 把渲染好的图表（`image_html`）包装成带「源码 ↔ 图片」切换按钮的居中容器。
///
/// - 默认展示渲染后的图片（`image_html`）；
/// - **左上角**有一个眼睛图标，**平时隐藏**，鼠标悬停到图片时才显现；
///   悬停到图标会悬浮出「View source / Show image」气泡提示（仿 mdbook 复制按钮）；
///   点击在「图片」与「源码」（`source`，已 HTML 转义）之间切换，再点回到图片；
///   图标放左上角，与 mdbook 右上角的代码复制按钮不重叠；
/// - 样式与 onclick 全部内联，生成后不依赖主题即可工作；`hidden` 属性由浏览器
///   原生支持，无需额外全局 JS。
///
/// 生成的 HTML 不含空行，pulldown-cmark 不会在中间截断该 HTML 块。
///
/// 内嵌内容（源码与 SVG）会先**去掉空行**：`<div>` 是 type-6 HTML 块，遇到空行即
/// 结束。若源码/SVG 里含空行（如 svgbob 的 `<style>` 多行 CSS 之间常有空行），
/// pulldown-cmark 会在该处提前截断容器，把 `</code></pre></div>` 误当成普通
/// Markdown 解析而报 “unexpected/unclosed HTML tag” 警告（与 echarts.rs 内嵌
/// TikZ/Typst 前先 `filter(!trim().is_empty())` 的约定一致）。
pub fn diagram_toggle_html(image_html: &str, source: &str) -> String {
    let normalized = source
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let escaped = escape_xml(&normalized);
    let raw = format!(
        "<div class=\"diagram-box\">\
<style>{css}</style>\
<button type=\"button\" class=\"diagram-toggle-btn\" data-tooltip=\"View source\" title=\"View source\" aria-label=\"View source\" \
onclick=\"var p=this.parentNode,q=p.querySelector('.diagram-toggle-source'),i=p.querySelector('.diagram-toggle-image');var s=q.hidden;i.hidden=s;q.hidden=!s;var t=s?'Show image':'View source';this.title=t;this.setAttribute('aria-label',t);this.setAttribute('data-tooltip',t)\">{eye}</button>\
<div class=\"diagram-toggle-image\">{image}</div>\
<pre class=\"diagram-toggle-source\" hidden><code>{source}</code></pre>\
</div>",
        css = DIAGRAM_TOGGLE_CSS,
        eye = DIAGRAM_TOGGLE_EYE_SVG,
        image = image_html,
        source = escaped
    );
    // 压缩整体输出中的全部空行：svgbob 等 SVG 的 `<style>` 内多行 CSS 之间的
    // 空行也在其中——保证整个容器是单条无空行的 type-6 块，pulldown-cmark 不会截断。
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

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
/// 强杀进程：Unix 用 SIGKILL；Windows 用 `taskkill /F /T`（`libc::kill` 为 POSIX API，Windows 不可用）
pub(crate) fn kill_process(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        // /T：连同子进程树一起终止（Chrome 会派生渲染子进程）
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .output();
    }
}

/// 优雅终止进程：Unix 用 SIGTERM；Windows 用 `taskkill`（不带 /F，先请求关闭）
pub(crate) fn terminate_process(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .output();
    }
}

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
    #[test]
    #[cfg(unix)]
    fn test_kill_process_terminates_child() {
        // spawn 一个长 sleep 子进程，kill 后应被 SIGKILL 终止
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let pid = child.id();
        kill_process(pid);
        let status = child.wait().unwrap();
        assert!(!status.success(), "子进程应被强制终止");
        assert!(status.code().is_none(), "被信号终止时无退出码");
    }
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
    fn test_diagram_toggle_html_structure() {
        let out = super::diagram_toggle_html("<img src=\"x.svg\" alt=\"tikz\">", "\\begin{tikzpicture}\\end{tikzpicture}");
        // 默认显示图片、隐藏源码
        assert!(out.contains("<div class=\"diagram-box\">"), "缺少外层容器: {}", out);
        assert!(out.contains("diagram-toggle-btn"), "缺少切换按钮: {}", out);
        assert!(out.contains("<div class=\"diagram-toggle-image\"><img src=\"x.svg\" alt=\"tikz\"></div>"), "图片视图缺省可见: {}", out);
        assert!(out.contains("<pre class=\"diagram-toggle-source\" hidden><code>"), "源码默认 hidden: {}", out);
        // 源码视图边框更柔和（更浅边框色 + 圆角增大 + 轻阴影），不再是粗重 rgba(...,.25)
        assert!(out.contains("border:1px solid rgba(127,127,127,.12)"), "源码框边框应柔和: {}", out);
        assert!(out.contains("border-radius:6px"), "源码框应圆角增大: {}", out);
        assert!(out.contains("box-shadow:0 1px 3px"), "源码框应加轻阴影: {}", out);
        // 源码必须被 HTML 转义，不能破坏外层结构
        assert!(out.contains("\\begin{tikzpicture}\\end{tikzpicture}"), "源码应保留: {}", out);
        assert!(out.ends_with("</div>"), "容器必须闭合: {}", out);
        // 内联样式含居中规则
        assert!(out.contains(".diagram-box{position:relative;text-align:center"), "缺少居中样式: {}", out);
        // 按钮在左上角（与 mdbook 右上角复制按钮错开）、无文字、只显示眼睛图标
        let btn_css = out[out.find("diagram-toggle-btn{").unwrap()..out.find(".diagram-toggle-btn svg").unwrap()].to_string();
        assert!(btn_css.contains("left:8px"), "按钮应在左上角: {}", btn_css);
        assert!(!btn_css.contains("right:8px"), "按钮不应在右上角（避免与复制按钮重叠）: {}", btn_css);
        // 平时隐藏（opacity/visibility 双隐藏），悬停容器才显现（仿 mdbook 复制按钮）
        assert!(btn_css.contains("opacity:0") && btn_css.contains("visibility:hidden"), "按钮应默认隐藏: {}", btn_css);
        assert!(out.contains(".diagram-box:hover .diagram-toggle-btn"), "缺少悬停显示规则: {}", out);
        // 用 :focus-visible 而非 :focus，避免点击后焦点残留一直显示
        assert!(out.contains(":focus-visible"), "缺少 focus-visible: {}", out);
        assert!(!out.contains("diagram-toggle-btn:focus{"), "不应用 :focus: {}", out);
        // 悬停按钮时悬浮文字气泡（仿 mdbook Copy to clipboard，文字为英文）
        assert!(out.contains("diagram-toggle-btn::after") && out.contains("attr(data-tooltip)"), "缺少悬浮提示气泡: {}", out);
        assert!(out.contains("data-tooltip=\"View source\""), "缺少 data-tooltip: {}", out);
        assert!(out.contains("title=\"View source\""), "按钮缺少 title: {}", out);
        assert!(!out.contains(">源码<") && !out.contains(">图片<"), "按钮不应有可见文字: {}", out);
        // 气泡位于按钮正上方（而非右侧）
        let after_css = out[out.find(".diagram-toggle-btn::after").unwrap()..].to_string();
        let a_end = after_css.find("}").unwrap();
        assert!(after_css[..a_end].contains("bottom:calc(100% + 8px)"), "气泡应在按钮正上方: {}", &after_css[..a_end]);
        // 按钮内含一个眼睛 <svg>
        let btn = &out[out.find("<button").unwrap()..];
        let btn_end = btn.find("</button>").unwrap();
        let btn_html = &btn[..btn_end];
        assert!(btn_html.contains("<svg"), "按钮缺少眼睛图标: {}", btn_html);
    }

    #[test]
    fn test_diagram_toggle_html_escapes_source() {
        let out = super::diagram_toggle_html("<img>", "a < b & c > d \"q\" 's'");
        assert!(out.contains("a &lt; b &amp; c &gt; d &quot;q&quot; &apos;s&apos;"), "源码未转义: {}", out);
    }

    #[test]
    fn test_diagram_toggle_html_strips_source_blank_lines() {
        // 源码含空行时，容器内不能出现空行（空行会让 pulldown-cmark 截断 HTML 块）
        let src = "\\begin{tikzpicture}\n\n\n\\draw (0,0) -- (1,1);\n\n\\end{tikzpicture}";
        let out = super::diagram_toggle_html("<svg/>", src);
        assert_ne!(out.contains("\n\n"), true, "容器内不应出现空行: {:?}", out);
        // 且源码内容仍完整保留（空行被移除）
        assert!(out.contains("\\begin{tikzpicture}"), "缺失首行: {}", out);
        assert!(out.contains("\\draw (0,0) -- (1,1);"), "缺失中间行: {}", out);
        assert!(out.contains("\\end{tikzpicture}"), "缺失末行: {}", out);
        assert!(out.ends_with("</div>"), "容器必须完整闭合: {}", out);
    }

    #[test]
    fn test_diagram_toggle_html_roundtrip_pulldown_cmark() {
        // 回归测试：切换容器经 pulldown-cmark 二次解析后，源码必须原样保留，
        // 不能被空行/行首特殊字符（如 svgbob SVG <style> 内多行 CSS 之间的空行、
        // pikchr 源码的 # 注释行）提前结束 type-6 HTML 块而打散成段/标题。
        use pulldown_cmark::{html, Options, Parser};
        // 模拟 svgbob 的 SVG：<style> 里 CSS 规则之间带空行，且含 <text>
        let image = "<div class=\"diagram-svgbob\"><svg><style>.x {\ncolor: red;\n}\n\n.x2 {\ncolor: blue;\n}</style><text>hi</text></svg></div>";
        let src = "box \"a\"\n# First row of objects\nbox \"b\"\n- item\n> quote";
        let out = super::diagram_toggle_html(image, src);
        // 输出必须无空行（type-6 块不被截断）
        assert!(!out.contains("\n\n"), "容器内含空行会被 pulldown-cmark 截断: {:?}", out);
        let mut buf = String::new();
        let parser = Parser::new_ext(&out, Options::all());
        html::push_html(&mut buf, parser);
        // 源码 + SVG <style> 内容原样保留
        assert!(buf.contains("# First row of objects"), "源码被打散: {}", buf);
        assert!(buf.contains("color: red;"), "SVG <style> 被打散: {}", buf);
        assert!(buf.contains("color: blue;"), "SVG <style> 被打散: {}", buf);
        // 不得被当成标题/段落
        assert!(!buf.contains("First row of objects</h1>"), "源码被解析成标题: {}", buf);
        assert!(!buf.contains("<p>color: red"), "SVG <style> 被解析成段落: {}", buf);
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
