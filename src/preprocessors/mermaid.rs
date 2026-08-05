//! mdbook-mermaid — Mermaid 图表预处理器
//!
//! 将 ```mermaid 代码块替换为 <div class="mermaid-container"> 标签。

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use regex::Regex;

pub struct MermaidPreprocessor;

impl Preprocessor for MermaidPreprocessor {
    fn name(&self) -> &str {
        "mdbook-mermaid"
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

fn process_chapter(content: &str) -> String {
    let re = Regex::new(r"(?ms)```\s*mermaid\s*\n(.*?)```").unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let diagram = caps.get(1).unwrap().as_str();
        // HTML 转义 <、& 等：mermaid 语法（如 "A9 <--> B9"）含 `<`，直接内联进
        // HTML 块会让 pulldown-cmark 的 HTML 解析器误判为标签开始，报
        // "Saw - in state TagOpen" 警告。浏览器解析 &lt; 后 mermaid.js 读取
        // textContent 得到原始字符，渲染不受影响。
        let escaped = crate::utils::escape_xml(diagram.trim());
        // 单行化：mermaid 代码块若位于列表项/段落内（如缩进代码块），多行 HTML
        // 会被 pulldown-cmark 拆断，产生 "unclosed HTML tag <div> while exiting
        // Item/Paragraph" 警告。把内容换行转为 &#10; 实体（浏览器 textContent
        // 会解码回换行，mermaid 渲染不受影响），整体保持单行。
        let escaped = escaped.replace('\n', "&#10;");
        format!("<div class=\"mermaid-container\" style=\"text-align: center\"><div class=\"mermaid\">{}</div></div>", escaped)
    }).to_string()
}

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
    process_chapter(content)
}

/// 运行 mdbook-mermaid 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = MermaidPreprocessor;
    crate::utils::run_preprocessor(&pre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_chapter_escapes_angle_brackets() {
        // mermaid 语法含 `<-->`（如 "A9 <--> B9"），必须转义避免 pulldown-cmark
        // 的 HTML 解析器误判为标签（"Saw - in state TagOpen" 警告）
        let md = "```mermaid\ngraph TB\n    A9 <--> B9\n```\n";
        let out = process_chapter(md);
        assert!(out.contains("&lt;--&gt;"), "尖括号未转义: {}", out);
        assert!(!out.contains("<-->"), "原始 <--> 残留: {}", out);
        assert!(out.starts_with("<div class=\"mermaid-container\""), "HTML 包装缺失: {}", out);
    }

    #[test]
    fn test_process_chapter_plain_diagram() {
        let md = "```mermaid\ngraph TB\n    a-->b\n```\n";
        let out = process_chapter(md);
        assert!(out.contains("graph TB"), "内容缺失: {}", out);
        assert!(out.contains("a--&gt;b") || out.contains("a--&gt;b"), "内容被破坏: {}", out);
        assert!(out.contains("</div></div>"), "闭合缺失: {}", out);
    }
}
