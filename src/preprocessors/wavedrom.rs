//! mdbook-wavedrom-rs — WaveDrom 时序图预处理器
//!
//! 将 ```wavedrom 代码块包裹在 <pre class="wavedrom"> 标签中。
//! 由前端 wavedrom.min.js 渲染为 SVG 时序图。

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use regex::Regex;

pub struct WavedromPreprocessor;

impl Preprocessor for WavedromPreprocessor {
    fn name(&self) -> &str {
        "mdbook-wavedrom-rs"
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
    // 匹配 ```wavedrom ... ``` 代码块
    let re = Regex::new(r"(?ms)```wavedrom\s*\n(.*?)```").unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let json = caps.get(1).unwrap().as_str().trim();
        format!("<pre class=\"wavedrom\">\n{json}\n</pre>\n")
    }).to_string()
}

/// 运行 mdbook-wavedrom-rs 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = WavedromPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
