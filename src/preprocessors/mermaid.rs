//! mdbook-mermaid — Mermaid 图表预处理器
//!
//! 将 ```mermaid 代码块替换为 <div class="mermaid-container"> 标签。

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use regex::Regex;

pub struct MermaidPreprocessor;

impl Preprocessor for MermaidPreprocessor {
    fn name(&self) -> &str {
        "mdbook-mermaid"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer != "not-supported"
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
        format!("<div class=\"mermaid-container\" style=\"text-align: center\"><div class=\"mermaid\">\n{}</div></div>\n", diagram.trim())
    }).to_string()
}

/// 运行 mdbook-mermaid 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = MermaidPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
