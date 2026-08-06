//! mdbook-pdf-preview — PDF 预览预处理器
//!
//! 功能：
//!   将 Markdown 中形如 `[text](./file.pdf "web-preview")` 的 PDF 引用语句
//!   （链接标题为 `web-preview`）替换为可交互的嵌入式 PDF 预览容器。
//!   其余 `.pdf` 链接保持普通链接，不做预览。
//!
//! 渲染：
//!   替换后 → 📄 占位区 → 滚动到视口 → PDFObject 内嵌浏览器原生 PDF viewer
//!   （assets/pdfviewer/pdfobject.min.js，仅 5KB；不支持原生 PDF 的浏览器
//!   显示提示与下载链接）。主题跟随由 pdf-preview.js 处理。
//!
//! 使用方式（book.toml）：
//! ```toml
//! [preprocessor.pdf-preview]
//! command = "mdbook-plugins pdf-preview"
//!
//! [output.html]
//! additional-js = [
//!     "./assets/pdfviewer/pdfobject.min.js",
//!     "./assets/pdfviewer/pdf-preview.js",
//! ]
//! ```

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use regex::Regex;

/// PDF 预览 CSS（内联注入）
const CSS_TEMPLATE: &str = include_str!("../../assets/pdfviewer/pdf-preview.css.html");

/// 正则：匹配 `[text](path/to/file.pdf "web-preview")`
/// 仅链接标题为 `web-preview` 的 PDF 引用才替换为内嵌预览，
/// 其余 `.pdf` 链接保持普通链接；不匹配图片引用 `![...](...)`。
static PDF_LINK_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
    Regex::new(r#"(?P<before>^|[^!])\[(?P<text>[^\]]*)\]\((?P<path>[^)]+\.pdf)\s+["']web-preview["']\)"#).unwrap()
});

pub struct PdfPreviewPreprocessor;

impl Preprocessor for PdfPreviewPreprocessor {
    fn name(&self) -> &str {
        "mdbook-pdf-preview"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer == "html")
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let mut error: Option<Error> = None;
        book.for_each_mut(|item: &mut BookItem| {
            if error.is_some() {
                return;
            }
            if let BookItem::Chapter(ref mut chapter) = *item {
                match Self::process_chapter(&chapter.content) {
                    Ok(content) => {
                        let mut new_content = String::new();
                        new_content.push_str(CSS_TEMPLATE);
                        new_content.push('\n');
                        new_content.push_str(&content);
                        chapter.content = new_content;
                    }
                    Err(e) => error = Some(e),
                }
            }
        });
        error.map_or(Ok(book), Err)
    }
}

impl PdfPreviewPreprocessor {
    fn process_chapter(content: &str) -> Result<String, Error> {
        let processed = PDF_LINK_RE.replace_all(content, |caps: &regex::Captures| {
            let before = &caps["before"];
            let _text = &caps["text"];
            let pdf_path = &caps["path"];
            let filename = pdf_path.rsplit('/').next().unwrap_or(pdf_path);
            // 单行化：多行 HTML 插入 markdown 会在列表项/段落内被 pulldown-cmark 拆断，
            // 产生 "unclosed HTML tag <div> while exiting Item/Paragraph" 警告。
            let pdf_path = pdf_path.replace('&', "&amp;").replace('"', "&quot;");
            let filename = filename.replace('&', "&amp;").replace('"', "&quot;");

            format!(
                r##"{before}<div class="pdfviewer-container" data-pdf-src="{pdf_path}"><div class="ppv-placeholder"><div class="ppv-icon">📄</div><div class="ppv-filename">{filename}</div><div class="ppv-hint">点击加载 PDF 预览</div></div></div>"##
            )
        });
        Ok(processed.to_string())
    }
}

/// 统一的处理入口：供测试复用（unified 未包含 pdf-preview）
pub fn process_content(content: &str) -> String {
    let processed = PDF_LINK_RE.replace_all(content, |caps: &regex::Captures| {
        let before = &caps["before"];
        let _text = &caps["text"];
        let pdf_path = &caps["path"];
        let filename = pdf_path.rsplit('/').next().unwrap_or(pdf_path);
        let pdf_path = pdf_path.replace('&', "&amp;").replace('"', "&quot;");
        let filename = filename.replace('&', "&amp;").replace('"', "&quot;");

        format!(
            r##"{before}<div class="pdfviewer-container" data-pdf-src="{pdf_path}"><div class="ppv-placeholder"><div class="ppv-icon">📄</div><div class="ppv-filename">{filename}</div><div class="ppv-hint">点击加载 PDF 预览</div></div></div>"##
        )
    });
    format!("{CSS_TEMPLATE}\n{processed}")
}

/// 运行 mdbook-pdf-preview 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = PdfPreviewPreprocessor;
    crate::utils::run_preprocessor(&pre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_preview_link_is_rendered() {
        let md = "参考[线性代数](./线性代数应该这样学-第四版.pdf \"web-preview\")文档\n";
        let out = process_content(md);
        assert!(
            out.contains("data-pdf-src="),
            "带 web-preview 标题的 PDF 链接应渲染为预览容器: {}",
            out
        );
        assert!(
            out.contains("data-pdf-src=\"./线性代数应该这样学-第四版.pdf\""),
            "预览容器应保留原始 pdf 路径: {}",
            out
        );
    }

    #[test]
    fn test_plain_pdf_link_not_rendered() {
        let md = "[产品手册](https://www.st.com/resource/en/datasheet/stm8s103f2.pdf) 和 [NIST](a.pdf)\n";
        let out = process_content(md);
        assert!(
            !out.contains("data-pdf-src="),
            "普通 .pdf 链接不应被渲染为预览容器: {}",
            out
        );
        // 原始链接保持原样
        assert!(out.contains("[产品手册](https://www.st.com/resource/en/datasheet/stm8s103f2.pdf)"));
    }

    #[test]
    fn test_single_quote_title_also_matches() {
        let md = "[x](./a.pdf 'web-preview')\n";
        assert!(
            process_content(md).contains("data-pdf-src="),
            "单引号 web-preview 标题也应匹配"
        );
    }

    #[test]
    fn test_other_title_not_rendered() {
        let md = "[x](./a.pdf \"other-title\")\n";
        assert!(
            !process_content(md).contains("data-pdf-src="),
            "非 web-preview 标题不应渲染"
        );
    }
}
