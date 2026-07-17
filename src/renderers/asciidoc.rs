//! mdbook-asciidoc — AsciiDoc 渲染器

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_renderer::Renderer;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::io::Write;
use std::path::Path;

pub struct AsciiDocRenderer;

impl Renderer for AsciiDocRenderer {
    fn name(&self) -> &str {
        "asciidoc"
    }

    fn render(&self, ctx: &mdbook_renderer::RenderContext) -> Result<(), Error> {
        let destination = ctx.destination.clone();
        let book = &ctx.book;

        if !destination.exists() {
            std::fs::create_dir_all(&destination)?;
        }

        if let Some(section) = book.iter().next() {
            process_section(section, &destination, book)?;
        } else {
            log::warn!("No sections found in the book");
        }

        Ok(())
    }
}

fn process_section(
    section: &BookItem,
    destination: &Path,
    _book: &Book,
) -> Result<(), Error> {
    match section {
        BookItem::Chapter(chapter) => {
            let adoc_content = markdown_to_asciidoc(&chapter.content);
            let file_path = destination.join(format!("{}.adoc", chapter.name));
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::File::create(&file_path)?;
            file.write_all(adoc_content.as_bytes())?;
            Ok(())
        }
        BookItem::Separator => Ok(()),
        BookItem::PartTitle(_) => Ok(()),
    }
}

fn markdown_to_asciidoc(markdown: &str) -> String {
    let parser = Parser::new(markdown);
    let mut adoc = String::new();
    let mut in_code_block = false;
    let mut _in_list = false;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let prefix = "=".repeat(level as usize);
                adoc.push_str(&format!("{} ", prefix));
            }
            Event::End(TagEnd::Heading(_)) => {
                adoc.push('\n');
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                adoc.push_str("\n\n");
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) => info.to_string(),
                    _ => String::new(),
                };
                adoc.push_str(&format!("[source,{}]\n----\n", lang));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                adoc.push_str("----\n\n");
            }
            Event::Start(Tag::List(_)) => {
                _in_list = true;
            }
            Event::End(TagEnd::List(_)) => {
                _in_list = false;
                adoc.push('\n');
            }
            Event::Start(Tag::Item) => {
                adoc.push_str("* ");
            }
            Event::End(TagEnd::Item) => {
                adoc.push('\n');
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                adoc.push_str(&dest_url.to_string());
                adoc.push('[');
            }
            Event::End(TagEnd::Link) => {
                adoc.push(']');
            }
            Event::Text(text) => {
                if in_code_block {
                    adoc.push_str(&text.replace('\n', "\n"));
                } else {
                    adoc.push_str(&text);
                }
            }
            Event::Code(text) => {
                adoc.push_str(&format!("`{}`", text));
            }
            Event::SoftBreak => {
                adoc.push('\n');
            }
            Event::HardBreak => {
                adoc.push_str(" +\n");
            }
            _ => {}
        }
    }
    adoc
}

/// 运行 mdbook-asciidoc 渲染器
pub fn run() -> anyhow::Result<()> {
    use mdbook_renderer::RenderContext;
    let mut ctx = RenderContext::from_json(std::io::stdin())?;
    let renderer = AsciiDocRenderer;
    renderer.render(&mut ctx)?;
    Ok(())
}
