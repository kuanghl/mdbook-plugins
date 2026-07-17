//! mdbook-admonish — Admonition 提示框预处理器
//!
//! 将 ```admonish <type> 代码块转换为 Material Design 风格的提示框。
//!
//! 支持语法：
//! ```admonish <type> [title="..."] [collapsible=true]
//! Content (Markdown)
//! ```
//!
//! 也支持 ~~~admonish fence 语法。
//!
//! 注意：采用字符串操作而非 pulldown-cmark 解析，以避免丢失 HTML 结构（如
//! 其他预处理器注入的 <style>/<script> 标签、标题、段落等结构事件）。
//!
//! CSS 样式通过 book.toml 中的 additional-css 外部加载（./theme/mdbook-admonish.css），
//! 不在预处理阶段内联注入，以保证输出与原始 mdbook-admonish 一致。

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use pulldown_cmark::{html, Options, Parser};
use std::collections::HashMap;

pub struct AdmonishPreprocessor;

impl Preprocessor for AdmonishPreprocessor {
    fn name(&self) -> &str {
        "mdbook-admonish"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer != "not-supported")
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                chapter.content = process_chapter(&chapter.content);
            }
        });
        Ok(book)
    }
}

/// 解析 admonish 起始行，返回 (类型, 自定义标题, 是否可折叠)
fn parse_admonish_start(line: &str) -> (String, Option<String>, bool) {
    // 去掉 ``` 或 ~~~ 前缀
    let rest = line
        .trim_start_matches("```")
        .trim_start_matches("~~~")
        .trim();
    // 去掉 "admonish" 关键字
    let rest = rest
        .strip_prefix("admonish")
        .unwrap_or(rest)
        .trim();

    // 解析参数
    let mut admonish_type = String::from("note");
    let mut custom_title: Option<String> = None;
    let mut collapsible = false;

    // 第一个 token 是类型（除非被引号包裹的标题在前）
    let mut tokens = rest.split_whitespace().peekable();

    // 如果第一个 token 是类型（不以 " 开头），则解析它
    if let Some(first) = tokens.peek() {
        if !first.starts_with('"') && !first.eq_ignore_ascii_case("collapsible") {
            admonish_type = first.to_string();
            tokens.next(); // 消费类型 token
        }
    }

    // 解析剩余参数
    let remaining: Vec<&str> = tokens.collect();
    let joined = remaining.join(" ");

    // 解析 collapsible 参数
    if joined.contains("collapsible") {
        collapsible = true;
    }

    // 提取标题：支持 title="xxx" 或 "xxx" 格式
    if let Some(start) = joined.find('"') {
        if let Some(end) = joined[start + 1..].find('"') {
            let title = &joined[start + 1..start + 1 + end];
            if !title.is_empty() {
                custom_title = Some(title.to_string());
            }
        }
    }

    (admonish_type, custom_title, collapsible)
}

/// 判断一行是否为 fence 代码块起始/结束标记（``` 或 ~~~）
fn is_fence_marker(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn process_chapter(content: &str) -> String {
    // 使用计数器跟踪每个类型的出现次数，生成与原始 mdbook-admonish 兼容的 ID
    let mut type_counter: HashMap<String, u32> = HashMap::new();

    // 结果字符串
    let mut output = String::with_capacity(content.len() + 1024);
    let mut in_admonish = false;
    let mut admonish_type = String::new();
    let mut custom_title: Option<String> = None;
    let mut collapsible = false;
    let mut admonish_lines: Vec<String> = Vec::new();
    let mut fence_char: &str = ""; // ``` 或 ~~~

    for line in content.lines() {
        let trimmed = line.trim();

        if in_admonish {
            // 查找代码块结束标记（与起始 fence 匹配）
            if trimmed == fence_char {
                // 结束 admonish 块
                let body = render_markdown(&admonish_lines.join("\n"));
                let display_title = custom_title
                    .clone()
                    .unwrap_or_else(|| capitalize_first(&admonish_type));
                // 将标题渲染为 Markdown（与原始 mdbook-admonish 行为一致）
                let rendered_title = render_inline_markdown(&display_title);

                // 生成 ID：自定义标题使用 slug 形式；自动标题使用 type 基础名加计数器
                let id = if custom_title.is_some() {
                    // 自定义标题：slug 化作为 ID
                    slugify_title(custom_title.as_ref().unwrap())
                } else {
                    // 自动标题：用 type 作为基础 ID，处理重复
                    let count = type_counter.entry(admonish_type.clone()).or_insert(0);
                    *count += 1;
                    if *count == 1 {
                        admonish_type.clone()
                    } else {
                        format!("{}-{}", admonish_type, *count - 1)
                    }
                };

                if collapsible {
                    let anchor = format!("#admonition-{}", id);
                    output.push_str(&format!(
                        r#"<details id="admonition-{id}" class="admonition admonish-{kind}">
<summary class="admonition-title">
<p>{title}</p>
<p><a class="admonition-anchor-link" href="{anchor}"></a></p>
</summary>
<div>
{body}
</div>
</details>
"#,
                        id = id,
                        anchor = anchor,
                        kind = admonish_type,
                        title = rendered_title,
                        body = body,
                    ));
                } else {
                    let anchor = format!("#admonition-{}", id);
                    output.push_str(&format!(
                        r#"<div id="admonition-{id}" class="admonition admonish-{kind}">
<div class="admonition-title">
<p>{title}</p>
<p><a class="admonition-anchor-link" href="{anchor}"></a></p>
</div>
<div>
{body}
</div>
</div>
"#,
                        id = id,
                        anchor = anchor,
                        kind = admonish_type,
                        title = rendered_title,
                        body = body,
                    ));
                }

                in_admonish = false;
                admonish_lines.clear();
                continue;
            }
            admonish_lines.push(line.to_string());
        } else if is_admonish_start(trimmed) {
            // 开始 admonish 块
            let (typ, title, coll) = parse_admonish_start(trimmed);
            admonish_type = typ;
            custom_title = title;
            collapsible = coll;

            // 确定 fence 字符
            if trimmed.trim_start().starts_with("~~~") {
                fence_char = "~~~";
            } else {
                fence_char = "```";
            }

            in_admonish = true;
            admonish_lines.clear();
        } else {
            // 普通行，原样输出
            output.push_str(line);
            output.push('\n');
        }
    }

    // 如果代码块未关闭，补回内容
    if in_admonish {
        output.push_str(&format!("{}admonish {}\n", fence_char, admonish_type));
        for line in &admonish_lines {
            output.push_str(line);
            output.push('\n');
        }
    }

    output
}

/// 首字母大写
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// 将字符串 slug 化为 HTML 锚点 ID（与原始 mdbook-admonish 兼容）
/// 先剥离 HTML 标签，然后 slug 化
fn slugify_title(text: &str) -> String {
    // 1) 剥离 HTML 标签
    let without_tags = strip_html_tags(text);
    // 2) 转小写并 slug 化：字母数字下划线保留，空白字符变连字符，其余移除
    let slug: String = without_tags
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c)
            } else if c.is_whitespace() {
                Some('-')
            } else {
                None // 移除其他字符（如撇号、逗号等）
            }
        })
        .flatten()
        .collect();

    // 3) 折叠连续空格/连字符并去头尾
    slug.split_whitespace()
        .collect::<Vec<&str>>()
        .join("-")
        .trim_matches('-')
        .to_string()
}

/// 剥离 HTML 标签（仅保留标签之间的文本内容）
fn strip_html_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
}

/// 检查一行是否为 admonish 起始标记
fn is_admonish_start(trimmed: &str) -> bool {
    // 匹配 ```admonish 或 ~~~admonish
    trimmed.starts_with("```admonish")
        || trimmed == "```admonish"
        || trimmed.starts_with("~~~admonish")
        || trimmed == "~~~admonish"
}

/// 使用 pulldown-cmark 将 Markdown 渲染为 HTML（与原始 mdbook-admonish 行为一致）
fn render_markdown(text: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(text, options);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output.trim().to_string()
}

/// 将 Markdown 渲染为内联 HTML（无 `<p>` 包裹），用于标题
fn render_inline_markdown(text: &str) -> String {
    let parser = Parser::new(text);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    let trimmed = output.trim();
    // 移除可能的 <p>...</p> 包裹
    if let Some(inner) = trimmed
        .strip_prefix("<p>")
        .and_then(|s| s.strip_suffix("</p>"))
    {
        inner.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
    process_chapter(content)
}

/// 运行 mdbook-admonish 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = AdmonishPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
