//! mdbook-langtabs — 语言标签页预处理器

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use regex::Regex;

pub struct LangTabsPreprocessor;

impl Preprocessor for LangTabsPreprocessor {
    fn name(&self) -> &str {
        "mdbook-langtabs"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer == "html")
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
    // 查找 <!-- langtabs-start --> 和 <!-- langtabs-end --> 包裹的区域
    let start_marker = "<!-- langtabs-start -->";
    let end_marker = "<!-- langtabs-end -->";

    let mut result = String::new();
    let mut remaining = content;
    let mut tab_id = 0u64;

    loop {
        match (remaining.find(start_marker), remaining.find(end_marker)) {
            (Some(start_pos), Some(end_pos)) if start_pos < end_pos => {
                // 添加 start 前的普通内容
                result.push_str(&remaining[..start_pos]);

                let block = &remaining[start_pos + start_marker.len()..end_pos];
                let tabs_html = render_tabs(block, &mut tab_id);
                result.push_str(&tabs_html);

                remaining = &remaining[end_pos + end_marker.len()..];
            }
            _ => {
                result.push_str(remaining);
                break;
            }
        }
    }
    result
}

fn render_tabs(block: &str, tab_id: &mut u64) -> String {
    // 解析代码块: ```<lang>\n...``` 
    let re = Regex::new(r"(?ms)```(\w+)\s*\n(.*?)```").unwrap();
    let mut tabs: Vec<(String, String)> = Vec::new();

    for cap in re.captures_iter(block) {
        let lang = cap.get(1).unwrap().as_str().to_string();
        let code = cap.get(2).unwrap().as_str().to_string();
        tabs.push((lang, code));
    }

    if tabs.is_empty() {
        return String::new();
    }

    let id = *tab_id;
    *tab_id += 1;

    let mut html = String::from("<div class=\"langtabs\">\n");

    // Tab 按钮
    html.push_str("<ul class=\"langtabs-tabs\" role=\"tablist\">\n");
    for (i, (lang, _)) in tabs.iter().enumerate() {
        let active = if i == 0 { " active" } else { "" };
        html.push_str(&format!(
            r#"  <li class="langtabs-tab{active}" role="tab" data-langtabs-tab="{id}-{i}">{lang}</li>"#,
        ));
        html.push('\n');
    }
    html.push_str("</ul>\n");

    // Tab 内容
    for (i, (_, code)) in tabs.iter().enumerate() {
        let active = if i == 0 { " active" } else { "" };
        html.push_str(&format!(
            r#"<div class="langtabs-panel{active}" role="tabpanel" data-langtabs-panel="{id}-{i}">
<pre><code>{}</code></pre>
</div>
"#,
            code.trim().escape_default()
        ));
    }

    html.push_str("</div>\n");
    html
}

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
    process_chapter(content)
}

/// 运行 mdbook-langtabs 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = LangTabsPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
