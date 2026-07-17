//! mdbook-toc — 目录生成预处理器

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::fmt::Write as _;

pub struct TocPreprocessor;

impl Preprocessor for TocPreprocessor {
    fn name(&self) -> &str {
        "mdbook-toc"
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
    // 查找 <!-- toc --> 标记
    let toc_marker = "<!-- toc -->";
    match content.find(toc_marker) {
        Some(pos) => {
            let before = &content[..pos + toc_marker.len()];
            let after = &content[pos + toc_marker.len()..];
            let toc = generate_toc(after);
            format!("{before}\n{toc}")
        }
        None => content.to_string(),
    }
}

fn generate_toc(content: &str) -> String {
    let parser = Parser::new(content);
    let mut toc = String::new();
    let mut in_heading = false;
    let mut heading_level = 0u8;
    let mut heading_text = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = true;
                heading_level = level as u8;
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if in_heading && !heading_text.is_empty() && heading_level <= 4 {
                    let indent = "  ".repeat((heading_level as usize).saturating_sub(1));
                    let slug = slugify(&heading_text);
                    let _ = writeln!(toc, "{indent}* [{heading_text}](#{slug})");
                }
                in_heading = false;
            }
                Event::Text(text) | Event::Code(text) => {
                if in_heading {
                    heading_text.push_str(&text);
                }
            }
            _ => {}
        }
    }
    toc
}

fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .fold(String::new(), |mut acc, c| {
            if c == '-' && acc.ends_with('-') {
                // skip duplicate hyphens
            } else {
                acc.push(c);
            }
            acc
        })
        .trim_matches('-')
        .to_string()
}

/// 运行 mdbook-toc 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = TocPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
