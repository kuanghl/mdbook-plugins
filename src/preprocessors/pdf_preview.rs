//! mdbook-pdf-preview — PDF 预览预处理器
//!
//! 功能：
//!   将 Markdown 中形如 `[text](./file.pdf)` 的 PDF 引用语句
//!   替换为可交互的嵌入式 PDF 预览容器。
//!
//! 效果：
//!   替换后 → 📄 占位区 → 点击 → Canvas 渲染 PDF（基于 pdf.js）
//!   支持主题跟随：阅读器背景随 mdbook 主题（Light/Coal/Ayu/Catppuccin 等）自动切换
//!
//! 使用方式（book.toml）：
//! ```toml
//! [preprocessor.pdf-preview]
//! command = "mdbook-plugins pdf-preview"
//!
//! [output.html]
//! additional-js = ["./assets/pdfviewer/pdf-preview.js"]
//! additional-css = ["./assets/pdfviewer/pdf-preview.css"]
//! ```
//!
//! 依赖：
//!   - assets/pdfviewer/pdf-preview.css.html（本文件 include_str! 内联注入）
//!   - test/assets/pdfviewer/build/pdf.js + pdf.worker.js（pdf.js 核心库，自动复制到输出）
//!   - test/assets/pdfviewer/pdf-preview.js（通过 additional-js 加载）

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use regex::Regex;

/// PDF 预览 CSS（内联注入）
const CSS_TEMPLATE: &str = include_str!("../../assets/pdfviewer/pdf-preview.css.html");

/// 正则：匹配 `[text](path/to/file.pdf)` 或 `[text](path/to/file.pdf/)`
/// 不匹配图片引用 `![...](...)`（前面没有 !）
static PDF_LINK_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
    Regex::new(r"(?P<before>^|[^!])\[(?P<text>[^\]]*)\]\((?P<path>[^)]+\.pdf)/?\)").unwrap()
});

pub struct PdfPreviewPreprocessor;

impl Preprocessor for PdfPreviewPreprocessor {
    fn name(&self) -> &str {
        "mdbook-pdf-preview"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer == "html")
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        // 自动复制 pdf.js 核心库到输出目录
        if let Err(e) = Self::copy_pdfjs_assets(ctx) {
            eprintln!("[pdf-preview] 警告: 复制 pdf.js 资源失败: {}", e);
        }

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
    /// 将 pdf.js 核心库（pdf.min.mjs + pdf.worker.min.mjs）从源代码复制到构建输出目录
    fn copy_pdfjs_assets(ctx: &PreprocessorContext) -> Result<(), Error> {
        let root = &ctx.root;
        // 构建输出目录：root / build_dir / html /
        let build_dir = &ctx.config.build.build_dir;
        let output_html = if build_dir.is_absolute() {
            build_dir.join("html")
        } else {
            root.join(build_dir).join("html")
        };

        let src_dir = root.join("assets").join("pdfviewer").join("build");
        let dst_dir = output_html.join("assets").join("pdfviewer").join("build");

        std::fs::create_dir_all(&dst_dir).map_err(|e| {
            Error::msg(format!("创建目录 {} 失败: {}", dst_dir.display(), e))
        })?;

        // 复制 pdf.min.mjs
        let src_js = src_dir.join("pdf.min.mjs");
        let dst_js = dst_dir.join("pdf.min.mjs");
        if src_js.exists() {
            std::fs::copy(&src_js, &dst_js).map_err(|e| {
                Error::msg(format!("复制 {} -> {} 失败: {}", src_js.display(), dst_js.display(), e))
            })?;
        }

        // 复制 pdf.worker.min.mjs
        let src_worker = src_dir.join("pdf.worker.min.mjs");
        let dst_worker = dst_dir.join("pdf.worker.min.mjs");
        if src_worker.exists() {
            std::fs::copy(&src_worker, &dst_worker).map_err(|e| {
                Error::msg(format!("复制 {} -> {} 失败: {}", src_worker.display(), dst_worker.display(), e))
            })?;
        }

        Ok(())
    }


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

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
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
    format!("{CSS_TEMPLATE}\n{processed}")
}

/// 运行 mdbook-pdf-preview 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = PdfPreviewPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
