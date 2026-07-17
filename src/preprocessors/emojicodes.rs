//! mdbook-emojicodes — Emoji shortcode 替换预处理器

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use regex::Regex;

pub struct EmojiCodesPreprocessor;

impl Preprocessor for EmojiCodesPreprocessor {
    fn name(&self) -> &str {
        "mdbook-emojicodes"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer != "not-supported"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                chapter.content = replace_emoji_shortcode(&chapter.content);
            }
        });
        Ok(book)
    }
}

fn replace_emoji_shortcode(text: &str) -> String {
    let mut inside_code_block = false;
    let mut result = String::with_capacity(text.len());
    let re = Regex::new(r":([^:\s]*?):").unwrap();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            inside_code_block = !inside_code_block;
            result.push_str(line);
        } else if !inside_code_block {
            let processed = re.replace_all(line, |caps: &regex::Captures| {
                let shortcode = caps.get(1).unwrap().as_str();
                match emojis::get_by_shortcode(shortcode) {
                    Some(emoji) => emoji.to_string(),
                    None => caps.get(0).unwrap().as_str().to_string(),
                }
            });
            result.push_str(&processed);
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    result
}

/// 运行 mdbook-emojicodes 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = EmojiCodesPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
