//! mdbook-linkcheck — 链接检查渲染器

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_renderer::{RenderContext, Renderer};
use regex::Regex;
use std::collections::HashSet;

const LINK_RE_STR: &str = r#"\[([^\]]+)\]\(([^)]+)\)"#;

pub struct LinkCheckRenderer;

impl Renderer for LinkCheckRenderer {
    fn name(&self) -> &str {
        "linkcheck"
    }

    fn render(&self, ctx: &RenderContext) -> Result<(), Error> {
        let book = &ctx.book;
        let mut all_links: HashSet<String> = HashSet::new();

        // 收集所有链接
        collect_links(book, &mut all_links);

        if all_links.is_empty() {
            log::info!("linkcheck: 没有发现需要检查的链接");
            return Ok(());
        }

        log::info!("linkcheck: 发现 {} 个链接需要检查", all_links.len());

        // 使用 tokio 运行时进行异步检查
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(check_links(&all_links))?;

        Ok(())
    }
}

fn collect_links(book: &Book, links: &mut HashSet<String>) {
    book.iter().for_each(|item| {
        if let BookItem::Chapter(ch) = item {
            let re = Regex::new(LINK_RE_STR).unwrap();
            for cap in re.captures_iter(&ch.content) {
                let url = cap.get(2).unwrap().as_str().to_string();
                // 跳过锚点链接和邮件链接
                if url.starts_with('#') || url.starts_with("mailto:") {
                    continue;
                }
                links.insert(url);
            }
        }
    });
}

async fn check_links(links: &HashSet<String>) -> Result<(), Error> {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("mdbook-linkcheck/0.1")
        .build()?;

    let mut handles = Vec::new();

    for url in links {
        let client = client.clone();
        let url = url.clone();

        // 跳过相对路径（本地文件）
        if !url.starts_with("http://") && !url.starts_with("https://") {
            log::debug!("linkcheck: 跳过本地路径: {}", url);
            continue;
        }

        let handle = tokio::spawn(async move {
            match client.head(&url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() || status.as_u16() == 405 {
                        // 405 Method Not Allowed 表示服务器不支持 HEAD，但 GET 可能可以
                        log::debug!("linkcheck: ✓ {} (HTTP {})", url, status);
                        Ok(())
                    } else {
                        // 对于 405, 尝试 GET
                        if status.as_u16() == 405 {
                            match client.get(&url).send().await {
                                Ok(get_resp) => {
                                    if get_resp.status().is_success() {
                                        log::debug!("linkcheck: ✓ {} (HTTP {} via GET)", url, get_resp.status());
                                        return Ok(());
                                    }
                                    Err(format!("✗ {} (HTTP {})", url, get_resp.status()))
                                }
                                Err(e) => Err(format!("✗ {} GET 失败: {}", url, e)),
                            }
                        } else {
                            Err(format!("✗ {} (HTTP {})", url, status))
                        }
                    }
                }
                Err(e) => Err(format!("✗ {} 请求失败: {}", url, e)),
            }
        });

        handles.push(handle);
    }

    let results = futures::future::join_all(handles).await;
    let mut errors = Vec::new();

    for result in results {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e),
            Err(e) => errors.push(format!("任务失败: {}", e)),
        }
    }

    if !errors.is_empty() {
        for err in &errors {
            log::error!("linkcheck: {}", err);
        }
        // 不返回错误，仅记录日志（避免中断构建）
        log::warn!("linkcheck: {} 个链接检查失败", errors.len());
    } else {
        log::info!("linkcheck: 所有链接检查通过");
    }

    Ok(())
}

/// 运行 mdbook-linkcheck 渲染器
pub fn run() -> anyhow::Result<()> {
    let renderer = LinkCheckRenderer;
    crate::utils::run_renderer(&renderer)
}
