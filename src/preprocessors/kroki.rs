//! mdbook-kroki-preprocessor — Kroki 图渲染预处理器
//!
//! 将 ```kroki 代码块发送到 Kroki 服务渲染为 SVG/PNG。

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use regex::Regex;

pub struct KrokiPreprocessor;

impl Preprocessor for KrokiPreprocessor {
    fn name(&self) -> &str {
        "mdbook-kroki-preprocessor"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer != "not-supported")
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        // Kroki 图缓存：{root}/{build_dir}/Kroki/ —— 内容未变化的图直接复用本地 SVG，
        // 避免 serve 反复重建时重复请求 Kroki 网络（一次构建 ~6.5s 的主要来源）。
        let cache_dir = ctx.root.join(&ctx.config.build.build_dir).join("Kroki");
        if std::fs::create_dir_all(&cache_dir).is_err() {
            log::warn!("kroki: 无法创建缓存目录 {}", cache_dir.display());
        }
        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                chapter.content = process_chapter(&chapter.content, Some(&cache_dir));
            }
        });
        Ok(book)
    }
}

fn process_chapter(content: &str, cache_dir: Option<&std::path::Path>) -> String {
    let re = Regex::new(r"(?ms)```kroki-(?P<type>\w+)\s*\n(?P<body>.*?)```").unwrap();
    let mut output = content.to_string();
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();

    for cap in re.captures_iter(content) {
        let diagram_type = cap.name("type").unwrap().as_str();
        let body = cap.name("body").unwrap().as_str();
        let start = cap.get(0).unwrap().start();
        let end = cap.get(0).unwrap().end();

        match render_kroki_sync(diagram_type, body, cache_dir) {
            Ok(svg) => {
                let wrapped = format!(
                    r#"<div class="kroki-wrapper kroki-{diagram_type}">{svg}</div>"#
                );
                replacements.push((start, end, wrapped));
            }
            Err(e) => {
                log::warn!("kroki 渲染 '{diagram_type}' 失败: {}", e);
                let fallback = format!(
                    r#"<pre><code class="language-kroki-{diagram_type}">{}</code></pre>"#,
                    body
                );
                replacements.push((start, end, fallback));
            }
        }
    }

    // 从后往前替换
    for (start, end, replacement) in replacements.into_iter().rev() {
        output.replace_range(start..end, &replacement);
    }

    output
}

fn render_kroki_sync(
    diagram_type: &str,
    body: &str,
    cache_dir: Option<&std::path::Path>,
) -> Result<String, Box<dyn std::error::Error>> {
    // 内容 hash 缓存：同一图内容直接复用本地 SVG，不重复请求网络
    let hash = crate::tikz::tikz_content_hash(body);
    let cache_file = cache_dir.map(|d| d.join(format!("kroki-{diagram_type}-{hash}.svg")));
    if let Some(f) = &cache_file {
        if f.is_file() {
            if let Ok(svg) = std::fs::read_to_string(f) {
                return Ok(svg);
            }
        }
    }

    let endpoint = std::env::var("KROKI_ENDPOINT")
        .unwrap_or_else(|_| "https://kroki.io".to_string());

    let body_encoded = base64_encode(body);
    let url = format!("{}/{}/svg/{}", endpoint, diagram_type, body_encoded);

    let resp = reqwest::blocking::get(&url)?;
    if !resp.status().is_success() {
        return Err(format!("Kroki API 返回 {}", resp.status()).into());
    }
    let svg = resp.text()?;
    // 写缓存（失败不影响本次渲染）
    if let Some(f) = &cache_file {
        let _ = std::fs::write(f, &svg);
    }
    Ok(svg)
}

fn base64_encode(input: &str) -> String {
    // 使用 deflate 压缩 + base64url 编码（Kroki 标准）
    use std::io::Write;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(input.as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&compressed)
}

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
    process_chapter(content, None)
}

/// 运行 mdbook-kroki-preprocessor 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = KrokiPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
