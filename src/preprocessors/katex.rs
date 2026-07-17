//! mdbook-katex — LaTeX 数学公式预处理器
//!
//! 将 `$...$`（行内）和 `$$...$$`（块级）LaTeX 公式
//! 转换为服务端预渲染的 KaTeX HTML（通过 node katex 模块），
//! 输出格式与原始 mdbook-katex 一致（含 <data class="katex-src"> 包装）。
//!
//! book.toml 需配置:
//! ```toml
//! [output.html]
//! additional-css = ["katex.min.css"]
//! ```

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use std::collections::HashMap;
use uuid::Uuid;

pub struct KatexPreprocessor;

impl Preprocessor for KatexPreprocessor {
    fn name(&self) -> &str {
        "mdbook-katex"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer != "not-supported"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let mut uuid_counter = 0u64;
        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                chapter.content = process_chapter(&chapter.content, &mut uuid_counter);
            }
        });
        Ok(book)
    }
}

fn process_chapter(content: &str, counter: &mut u64) -> String {
    // 1) 保护 <script> 和 <style> 块
    let (protected, mut placeholders) = protect_html_blocks(content);

    // 2) 保护 ``` 代码块
    let (protected2, mut code_placeholders) = protect_code_blocks(&protected);

    // 3) 先处理块级公式 $$...$$，再处理行内公式 $...$
    let processed = process_display_math(&protected2, counter);
    let processed = process_inline_math(&processed, counter);

    // 4) 恢复代码块（后保护的先恢复）
    let restored = restore_blocks(&processed, &mut code_placeholders);

    // 5) 恢复 HTML 块
    restore_blocks(&restored, &mut placeholders)
}

/// 用 UUID 占位符替换受保护的内容块
fn protect_block(content: &str, start: usize, end: usize, map: &mut HashMap<String, String>) -> String {
    let uuid = Uuid::new_v4().to_string().replace('-', "");
    let key = format!("\x01KPH_{}\x01", uuid);
    let block = &content[start..end];
    map.insert(key.clone(), block.to_string());
    let mut result = content[..start].to_string();
    result.push_str(&key);
    result.push_str(&content[end..]);
    result
}

/// 保护 ``` 代码块
fn protect_code_blocks(content: &str) -> (String, HashMap<String, String>) {
    let mut placeholders = HashMap::new();
    let mut s = content.to_string();
    let mut bytes = s.as_bytes().to_vec();

    loop {
        let mut found = false;
        let mut i = 0;
        while i + 3 <= bytes.len() {
            if &bytes[i..i+3] == b"```" {
                // 找到 info 行尾
                let mut info_end = i + 3;
                while info_end < bytes.len() && bytes[info_end] != b'\n' {
                    info_end += 1;
                }
                let content_start = if info_end < bytes.len() { info_end + 1 } else { info_end };
                // 找闭合 ```
                let mut close_pos = 0;
                let mut j = content_start;
                while j + 3 <= bytes.len() {
                    if (j == 0 || bytes[j-1] == b'\n') && &bytes[j..j+3] == b"```" {
                        close_pos = j + 3;
                        break;
                    }
                    j += 1;
                }
                let block_end = if close_pos > 0 {
                    let mut end = close_pos;
                    while end < bytes.len() && (bytes[end] == b'\n' || bytes[end] == b'\r') {
                        end += 1;
                    }
                    end
                } else {
                    bytes.len()
                };
                s = protect_block(&s, i, block_end, &mut placeholders);
                bytes = s.as_bytes().to_vec();
                found = true;
                break;
            }
            i += 1;
        }
        if !found {
            break;
        }
    }
    (s, placeholders)
}

/// 保护 <script>、<style>、<details> 块
fn protect_html_blocks(content: &str) -> (String, HashMap<String, String>) {
    let mut placeholders = HashMap::new();
    let mut s = content.to_string();
    let tags = [("script", "/script>"), ("style", "/style>")];

    loop {
        let lower = s.to_lowercase();
        let mut best_start = usize::MAX;
        let mut best_tag_end = "";
        let mut best_tag_name = "";

        for (tag_name, tag_end) in &tags {
            if let Some(pos) = lower.find(&format!("<{}", tag_name)) {
                if pos < best_start {
                    best_start = pos;
                    best_tag_end = tag_end;
                    best_tag_name = tag_name;
                }
            }
        }

        if best_start == usize::MAX {
            break;
        }

        // 验证是否是完整标签（以 > 或空格结束）
        let after_start = &s[best_start..];
        let tag_open_end = after_start.find('>').unwrap_or(after_start.len());
        let tag_open = &after_start[..=tag_open_end].to_lowercase();
        if !tag_open.starts_with(&format!("<{}", best_tag_name)) {
            // 不是期望的标签，跳过
            s = s.replacen(&after_start[..1], "", 1);
            continue;
        }

        // 找结束标签
        let after = &s[best_start..];
        let after_lower = after.to_lowercase();
        if let Some(end_pos) = after_lower.find(best_tag_end) {
            let block_end = best_start + end_pos + best_tag_end.len();
            s = protect_block(&s, best_start, block_end, &mut placeholders);
        } else {
            break;
        }
    }

    (s, placeholders)
}

/// 恢复所有占位符
fn restore_blocks(content: &str, placeholders: &mut HashMap<String, String>) -> String {
    let mut result = content.to_string();
    let entries: Vec<(String, String)> = placeholders.drain().collect();
    let mut sorted = entries;
    sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (key, block) in sorted {
        result = result.replace(&key, &block);
    }
    result
}

/// 通过 node katex 渲染 LaTeX 为 HTML（服务端渲染）
fn render_katex(latex: &str, display_mode: bool) -> String {
    let display_flag = if display_mode { "true" } else { "false" };
    // 使用 output:'html' 避免 MathML 输出（匹配原始 mdbook-katex v0.5.9 行为）
    let js_code = format!(
        r##"const k=require('katex');process.stdout.write(k.renderToString({},{{displayMode:{},throwOnError:false,output:'html'}}))"##,
        serde_json::Value::String(latex.to_string()),
        display_flag
    );
    let output = std::process::Command::new("node")
        .args(["-e", &js_code])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let html = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // 移除 inline style 中的 margin 属性以匹配旧版输出
            // katex-rs 0.12 版本的输出和 node katex 可能略有不同，
            // 我们保留 node katex 的输出
            html
        },
        _ => {
            // 渲染失败，原样输出 LaTeX
            if display_mode {
                format!("$${}$$", latex)
            } else {
                format!("${}$", latex)
            }
        }
    }
}

/// 处理块级数学公式 $$...$$
fn process_display_math(content: &str, counter: &mut u64) -> String {
    let mut result = String::new();
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'$') {
            chars.next();
            let mut inner = String::new();
            let mut closed = false;
            while let Some(ic) = chars.next() {
                if ic == '$' && chars.peek() == Some(&'$') {
                    chars.next();
                    closed = true;
                    break;
                }
                inner.push(ic);
            }
            if closed && !inner.is_empty() {
                *counter += 1;
                let katex_html = render_katex(&inner, true);
                // 编码换行符为 &#10;（匹配原始 mdbook-katex 行为）
                let latex_escaped = inner.replace('"', "&quot;").replace('\n', "&#10;");
                result.push_str(&format!(
                    r##"<data class="katex-src" value="{latex}">{html}</data>"##,
                    latex = latex_escaped,
                    html = katex_html,
                ));
            } else {
                result.push_str("$$");
                result.push_str(&inner);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// 处理行内数学公式 $...$
fn process_inline_math(content: &str, counter: &mut u64) -> String {
    let mut result = String::new();
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() != Some(&'$') {
            // 检查后续字符：如果 $ 后是 {、空白、数字等，不是数学公式
            let is_math_start = match chars.peek() {
                None | Some('{') | Some(' ') | Some('\t') | Some('\n')
                | Some('0'..='9') | Some(')') | Some('(')
                | Some('[') | Some(']') | Some('<') | Some('>')
                | Some(',') | Some('.') | Some(';') | Some(':')
                | Some('!') | Some('?') | Some('\'') | Some('"') => false,
                _ => true,
            };

            if is_math_start {
                let mut inner = String::new();
                let mut closed = false;
                for ic in chars.by_ref() {
                    if ic == '$' {
                        closed = true;
                        break;
                    }
                    if ic == '\n' {
                        break;
                    }
                    inner.push(ic);
                }
                if closed && !inner.is_empty() {
                    *counter += 1;
                    let katex_html = render_katex(&inner, false);
                    let latex_escaped = inner.replace('"', "&quot;").replace('\n', "&#10;");
                    result.push_str(&format!(
                        r##"<data class="katex-src" value="{latex}">{html}</data>"##,
                        latex = latex_escaped,
                        html = katex_html,
                    ));
                } else {
                    result.push('$');
                    result.push_str(&inner);
                    // 如果因换行中断，补回换行符
                    if !closed {
                        result.push('\n');
                    }
                }
            } else {
                // 不是数学公式，原样输出 $
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// 运行 mdbook-katex 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = KatexPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
