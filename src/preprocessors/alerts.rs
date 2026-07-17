//! mdbook-alerts — GitHub 风格 Alert 预处理器

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::PreprocessorContext;
use once_cell::sync::Lazy;
use regex::Regex;

/// Alerts CSS 样式（内嵌）
const STYLE_CSS: &str = include_str!("../../assets/alerts/style.css");

/// Alerts HTML 模板（内嵌）
const ALERTS_TMPL: &str =
    include_str!("../../assets/alerts/alerts.tmpl");

pub struct AlertsPreprocessor;

impl mdbook::preprocess::Preprocessor for AlertsPreprocessor {
    fn name(&self) -> &str {
        "mdbook-alerts"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer == "html"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let mut error: Option<Error> = None;
        book.for_each_mut(|item: &mut BookItem| {
            if error.is_some() {
                return;
            }
            if let BookItem::Chapter(ref mut chapter) = *item {
                if let Err(err) = handle_chapter(chapter) {
                    error = Some(err)
                }
            }
        });
        error.map_or(Ok(book), Err)
    }
}

fn handle_chapter(chapter: &mut mdbook::book::Chapter) -> Result<(), Error> {
    chapter.content = inject_stylesheet(&chapter.content)?;
    chapter.content = render_alerts(&chapter.content)?;
    Ok(())
}

fn inject_stylesheet(content: &str) -> Result<String, Error> {
    Ok(format!("<style>\n{STYLE_CSS}\n</style>\n{content}"))
}

fn render_alerts(content: &str) -> Result<String, Error> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?m)^> \[!(?P<kind>[^\]]+)\]\s*$(?P<body>(?:\n>.*)*)")
            .expect("failed to parse regex")
    });

    let tmpl = ALERTS_TMPL.replace("\r\n", "\n");
    let newline = find_newline(content);
    let normalized = content.replace(&newline, "\n");
    let result = RE.replace_all(&normalized, |caps: &regex::Captures| {
        let kind = caps
            .name("kind")
            .expect("kind not found")
            .as_str()
            .to_lowercase();
        let body = caps
            .name("body")
            .expect("body not found")
            .as_str()
            .replace("\n>\n", "\n\n")
            .replace("\n> ", "\n");
        tmpl.replace("{kind}", &kind).replace("{body}", &body)
    });
    Ok(result.replace('\n', &newline))
}

fn find_newline(content: &str) -> &'static str {
    let mut cr = 0;
    let mut lf = 0;
    content.chars().for_each(|c| match c {
        '\r' => cr += 1,
        '\n' => lf += 1,
        _ => {}
    });
    if cr == lf { "\r\n" } else { "\n" }
}

/// 运行 mdbook-alerts 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = AlertsPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
