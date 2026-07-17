//! mdbook-image-viewer — 图片查看器预处理器
//!
//! 将 Markdown 中的 `![alt](path)` 图片标记替换为可点击放大的 HTML，
//! 并注入模态框查看器的 CSS/JS。
//!
//! 效果：点击图片 → 模态框放大显示 → 支持拖拽/滚轮缩放/触控双指缩放

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use regex::Regex;

/// 模态框 CSS 样式（内嵌）
/// 模态框 CSS 样式（内嵌）
const CSS_TEMPLATE: &str = include_str!("../../assets/image-viewer.css.html");
/// 模态框 JS 脚本（内嵌）
const JS_TEMPLATE: &str = include_str!("../../assets/image-viewer.js.html");

pub struct ImageViewerPreprocessor;

impl Preprocessor for ImageViewerPreprocessor {
    fn name(&self) -> &str {
        "mdbook-image-viewer"
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
                match self.process_chapter(&chapter.content) {
                    Ok(content) => {
                        let mut new_content = String::new();
                        new_content.push_str(CSS_TEMPLATE);
                        new_content.push('\n');
                        new_content.push_str(&content);
                        new_content.push('\n');
                        new_content.push_str(JS_TEMPLATE);
                        chapter.content = new_content;
                    }
                    Err(e) => error = Some(e),
                }
            }
        });
        error.map_or(Ok(book), Err)
    }
}

impl ImageViewerPreprocessor {
    fn process_chapter(&self, content: &str) -> Result<String, Error> {
        let img_regex = Regex::new(r"!\[(.*?)\]\((.*?)\)").map_err(|e| {
            mdbook_core::errors::Error::msg(format!("regex error: {}", e))
        })?;
        let processed = img_regex.replace_all(content, |caps: &regex::Captures| {
            let alt_text = &caps[1];
            let image_path = &caps[2];
            format!(
                r#"<img src="{}" alt="{}" class="miv_mdbook-image-viewer" onclick="miv_openModal(this.src)">"#,
                image_path, alt_text
            )
        });
        Ok(processed.to_string())
    }
}

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
    let img_regex = Regex::new(r"!\[(.*?)\]\((.*?)\)").unwrap();
    let processed = img_regex.replace_all(content, |caps: &regex::Captures| {
        let alt_text = &caps[1];
        let image_path = &caps[2];
        format!(
            r##"<img src="{}" alt="{}" class="miv_mdbook-image-viewer" onclick="miv_openModal(this.src)">"##,
            image_path, alt_text
        )
    });
    format!("{CSS_TEMPLATE}
{processed}
{JS_TEMPLATE}")
}

/// 运行 mdbook-image-viewer 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = ImageViewerPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
