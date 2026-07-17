//! mdbook-embedify — 嵌入内容预处理器
//!
//! 将 {% <app> <options> %} 标签转换为嵌入式 HTML。

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};
use regex::Regex;

pub struct EmbedifyPreprocessor;

impl Preprocessor for EmbedifyPreprocessor {
    fn name(&self) -> &str {
        "mdbook-embedify"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer == "html"
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
    let content = replace_youtube(content);
    let content = replace_codepen(&content);
    let content = replace_giscus(&content);
    content
}

fn replace_youtube(content: &str) -> String {
    let re = Regex::new(r#"(?ms)\{%\s*youtube\s+["']?(?P<id>[a-zA-Z0-9_-]+)["']?\s*%}"#).unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let id = caps.name("id").unwrap().as_str();
        format!(
            r#"<div class="embedify-wrapper embedify-youtube">
<iframe width="560" height="315" src="https://www.youtube.com/embed/{id}" 
frameborder="0" allowfullscreen></iframe>
</div>"#
        )
    }).to_string()
}

fn replace_codepen(content: &str) -> String {
    let re = Regex::new(
        r#"(?ms)\{%\s*codepen\s+(?P<user>[^\s]+)\s+(?P<slug>[^\s]+)\s*(?:default-tab=(?P<tab>[^\s]+))?\s*%}"#,
    ).unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let user = caps.name("user").unwrap().as_str();
        let slug = caps.name("slug").unwrap().as_str();
        let tab = caps.name("tab").map(|m| m.as_str()).unwrap_or("result");
        format!(
            r#"<div class="embedify-wrapper embedify-codepen">
<iframe height="300" style="width: 100%;" scrolling="no" 
src="https://codepen.io/{user}/embed/{slug}?default-tab={tab}" 
frameborder="no" loading="lazy"></iframe>
</div>"#
        )
    }).to_string()
}

fn replace_giscus(content: &str) -> String {
    let re = Regex::new(
        r#"(?ms)\{%\s*giscus\s+repo=(?P<repo>[^\s%]+)\s+repo-id=(?P<repo_id>[^\s%]+)\s+category=(?P<cat>[^\s%]+)\s*%}"#,
    ).unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let repo = caps.name("repo").unwrap().as_str();
        let repo_id = caps.name("repo_id").unwrap().as_str();
        let cat = caps.name("cat").unwrap().as_str();
        format!(
            r#"<div class="embedify-wrapper embedify-giscus">
<script src="https://giscus.app/client.js"
data-repo="{repo}"
data-repo-id="{repo_id}"
data-category="{cat}"
data-loading="lazy"
crossorigin="anonymous"
async>
</script>
</div>"#
        )
    }).to_string()
}

/// 运行 mdbook-embedify 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = EmbedifyPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
