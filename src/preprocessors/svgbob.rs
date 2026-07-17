//! mdbook-svgbob — ASCII art 转 SVG 预处理器

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::fmt::Write;

pub struct SvgbobPreprocessor;

impl Preprocessor for SvgbobPreprocessor {
    fn name(&self) -> &str {
        "mdbook-svgbob"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer != "not-supported"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let mut error: Option<Error> = None;
        book.for_each_mut(|item: &mut BookItem| {
            if error.is_some() {
                return;
            }
            if let BookItem::Chapter(ref mut chapter) = item {
                match process_chapter(&chapter.content) {
                    Ok(content) => chapter.content = content,
                    Err(e) => error = Some(e),
                }
            }
        });
        error.map_or(Ok(book), Err)
    }
}

fn process_chapter(content: &str) -> Result<String, Error> {
    let parser = Parser::new(content);
    let mut output = String::new();
    let mut in_bob_block = false;
    let mut bob_lines = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let info = match &kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) => info.to_string(),
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
                if info.trim() == "bob" {
                    in_bob_block = true;
                    bob_lines.clear();
                    continue;
                }
                // 传递其他代码块
                in_bob_block = false;
                let _ = write!(output, "```{}\n", info);
            }
            Event::End(TagEnd::CodeBlock) => {
                if in_bob_block {
                    in_bob_block = false;
                    // 使用 svgbob crate 渲染 SVG
                    match render_svg(&bob_lines) {
                        Ok(svg) => {
                            output.push_str(&svg);
                            output.push('\n');
                        }
                        Err(e) => {
                            log::warn!("svgbob 渲染失败: {}", e);
                            let _ = write!(
                                output,
                                "<pre><code class=\"language-bob\">{}</code></pre>\n",
                                bob_lines
                            );
                        }
                    }
                } else {
                    output.push_str("```\n\n");
                }
            }
            Event::Text(text) => {
                if in_bob_block {
                    bob_lines.push_str(&text);
                } else {
                    output.push_str(&text);
                }
            }
            Event::Code(text) => {
                output.push_str(&format!("`{}`", text));
            }
            Event::SoftBreak => output.push('\n'),
            Event::HardBreak => output.push_str("  \n"),
            _ => {}
        }
    }
    Ok(output)
}

fn render_svg(ascii: &str) -> Result<String, Box<dyn std::error::Error>> {
    let settings = svgbob::Settings {
        // 提供合理的默认设置
        ..Default::default()
    };
    let svg_str = svgbob::to_svg_with_settings(ascii, &settings);
    Ok(svg_str)
}

/// 运行 mdbook-svgbob 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = SvgbobPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
