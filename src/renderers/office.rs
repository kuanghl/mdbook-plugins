//! mdbook-office — Office 文档渲染器
//!
//! 将 mdbook 内容渲染为 DOCX / XLSX / PPTX 格式。
//! 需要安装 Chrome/Chromium 以渲染图表（Mermaid、ECharts、KaTeX）。
//!
//! book.toml 配置:
//! ```toml
//! [output.office]
//! formats = ["docx", "xlsx", "pptx"]
//! # browser-path = "/usr/bin/chromium-browser"
//! ```

use mdbook::book::BookItem;
use mdbook::errors::Error;
use mdbook::renderer::{RenderContext, Renderer};
use std::path::{Path, PathBuf};

pub struct OfficeRenderer;

impl Renderer for OfficeRenderer {
    fn name(&self) -> &str {
        "office"
    }

    fn render(&self, ctx: &RenderContext) -> Result<(), Error> {
        run_office(ctx)
    }
}

fn run_office(ctx: &RenderContext) -> Result<(), Error> {
    let dest = &ctx.destination;
    std::fs::create_dir_all(dest)?;

    // 将配置转为 serde_json::Value 以便使用 pointer
    let cfg: serde_json::Value = serde_json::to_value(&ctx.config)?;
    let fmts = get_formats(&cfg);
    if fmts.is_empty() {
        return Ok(());
    }

    let raw = collect_chapters(&ctx.book.sections);
    if raw.is_empty() {
        return Ok(());
    }

    // 清理 HTML 标签，保留纯文本/Markdown 结构
    let text: Vec<(String, String)> = raw
        .iter()
        .map(|(n, c)| (n.clone(), clean_text(c)))
        .collect();

    // 需要 Chrome 渲染图表（docx/pptx）
    let need_chrome = fmts.iter().any(|f| f == "docx" || f == "pptx");
    let _diags = if need_chrome {
        match capture_diagrams(&raw, &ctx.root) {
            Ok(d) => {
                log::info!("mdbook-office: captured {} diagrams via Chrome", d.len());
                d
            }
            Err(e) => {
                log::warn!("mdbook-office: Chrome capture skipped ({})", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    for f in &fmts {
        match f.as_str() {
            "docx" => {
                let p = dest.join("book.docx");
                build_docx(&text, &p)?;
                log::info!("mdbook-office: docx ({})", fsize(&p));
            }
            "xlsx" => {
                let p = dest.join("book.xlsx");
                build_xlsx(&text, &p)?;
                log::info!("mdbook-office: xlsx ({})", fsize(&p));
            }
            "pptx" => {
                let p = dest.join("book.pptx");
                build_pptx(&text, &p)?;
                log::info!("mdbook-office: pptx ({})", fsize(&p));
            }
            _ => log::warn!("mdbook-office: unsupported format \"{f}\""),
        }
    }
    Ok(())
}

// =========================================================================
// 配置读取
// =========================================================================

fn get_formats(config: &serde_json::Value) -> Vec<String> {
    config
        .pointer("/output/office/formats")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec!["docx".into(), "xlsx".into(), "pptx".into()])
}

// 获取浏览器路径（暂未启用 Chrome 截图）
#[allow(dead_code)]
fn get_browser_path(config: &serde_json::Value) -> Option<PathBuf> {
    config
        .pointer("/output/office/browser-path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

// =========================================================================
// Chapter 收集
// =========================================================================

fn collect_chapters(sections: &[BookItem]) -> Vec<(String, String)> {
    let mut ch = Vec::new();
    for item in sections {
        match item {
            BookItem::Chapter(c) => {
                ch.push((c.name.clone(), c.content.clone()));
                collect_sub(&c.sub_items, &mut ch);
            }
            BookItem::PartTitle(t) => ch.push((t.clone(), format!("# {}\n", t))),
            BookItem::Separator => {}
        }
    }
    ch
}

fn collect_sub(items: &[BookItem], out: &mut Vec<(String, String)>) {
    for item in items {
        if let BookItem::Chapter(c) = item {
            out.push((c.name.clone(), c.content.clone()));
            collect_sub(&c.sub_items, out);
        }
    }
}

// =========================================================================
// HTML 清理
// =========================================================================

fn clean_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut in_tag = false;
    let mut skip_block = false;

    while i < n {
        if chars[i] == '<' {
            in_tag = true;
            let lower: String = chars
                .iter()
                .skip(i)
                .take(30)
                .collect::<String>()
                .to_lowercase();
            if lower.starts_with("<script")
                || lower.starts_with("<style")
                || lower.starts_with("<div class=\"mermaid")
                || lower.starts_with("<div class=\"echarts")
                || lower.starts_with("<span class=\"katex")
            {
                skip_block = true;
            }
            i += 1;
            continue;
        }
        if chars[i] == '>' {
            in_tag = false;
            if skip_block
                && (chars
                    .get(i.saturating_sub(8)..i)
                    .map(|s| s.iter().collect::<String>())
                    .unwrap_or_default()
                    .contains("</"))
            {
                // 粗略检测块结束
                let context: String = chars
                    .iter()
                    .skip(if i > 20 { i - 20 } else { 0 })
                    .take(30)
                    .collect();
                if context.contains("</script>")
                    || context.contains("</style>")
                    || context.contains("</div>")
                {
                    skip_block = false;
                }
            }
            let context: String = chars
                .iter()
                .skip(if i > 20 { i - 20 } else { 0 })
                .take(30)
                .collect();
            if context.contains("</div>")
                || context.contains("</p>")
                || context.contains("</h1>")
                || context.contains("</h2>")
                || context.contains("</h3>")
                || context.contains("</li>")
                || context.contains("</tr>")
            {
                out.push('\n');
            }
            i += 1;
            continue;
        }
        if in_tag || skip_block {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // 合并连续换行
    let mut cleaned = String::new();
    let mut blank_count = 0u32;
    for c in out.chars() {
        if c == '\n' {
            blank_count += 1;
            if blank_count <= 2 {
                cleaned.push(c);
            }
        } else if c != '\r' {
            blank_count = 0;
            cleaned.push(c);
        }
    }
    cleaned.trim().to_string()
}

// =========================================================================
// 简单的占位渲染（不使用 Chrome 时用 office_oxide 纯文本转换）
// =========================================================================

fn build_docx(ch: &[(String, String)], path: &Path) -> Result<(), Error> {
    let md = combine_md(ch);
    office_oxide::create::create_from_markdown(
        &md,
        office_oxide::format::DocumentFormat::Docx,
        path,
    )?;
    Ok(())
}

fn build_pptx(ch: &[(String, String)], path: &Path) -> Result<(), Error> {
    let md = combine_md(ch);
    office_oxide::create::create_from_markdown(
        &md,
        office_oxide::format::DocumentFormat::Pptx,
        path,
    )?;
    Ok(())
}

fn build_xlsx(ch: &[(String, String)], path: &Path) -> Result<(), Error> {
    let md = combine_md(ch);
    office_oxide::create::create_from_markdown(
        &md,
        office_oxide::format::DocumentFormat::Xlsx,
        path,
    )?;
    Ok(())
}

fn combine_md(ch: &[(String, String)]) -> String {
    let mut md = String::new();
    for (n, t) in ch {
        let tr = t.trim();
        if tr.is_empty() {
            md.push_str(&format!("# {}\n\n", n));
        } else if !tr.starts_with('#') {
            md.push_str(&format!("# {}\n\n{}", n, t));
        } else {
            md.push_str(t);
        }
        md.push_str("\n\n");
    }
    md
}

// =========================================================================
// Chrome 截图捕获（简化版）
// =========================================================================

struct DiagramCapture {
    _data: Vec<u8>,
    _kind: &'static str,
}

fn capture_diagrams(
    _chapters: &[(String, String)],
    _root: &Path,
) -> Result<Vec<DiagramCapture>, Box<dyn std::error::Error>> {
    // 桌面端完整版使用 headless_chrome crate
    // 为减少依赖复杂度，此简化版本跳过 Chrome 截图
    // 如需完整图表渲染，取消下面注释并添加 headless_chrome 依赖
    /*
    let browser_path = resolve_chrome(browser_path)?;
    let browser = Browser::new(
        LaunchOptions::default_builder()
            .headless(true)
            .sandbox(false)
            .path(Some(browser_path))
            .build()?
    )?;
    let tab = browser.new_tab()?;
    // ... navigate and capture ...
    */
    log::warn!("mdbook-office: Chrome diagram capture skipped (use headless_chrome feature for full support)");
    Ok(Vec::new())
}

fn fsize(path: &Path) -> String {
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    const K: u64 = 1024;
    const M: u64 = K * 1024;
    if len >= M {
        format!("{:.1} MB", len as f64 / M as f64)
    } else if len >= K {
        format!("{:.1} KB", len as f64 / K as f64)
    } else {
        format!("{} B", len)
    }
}

/// 运行 mdbook-office 渲染器
pub fn run() -> anyhow::Result<()> {
    let renderer = OfficeRenderer;
    crate::utils::run_renderer(&renderer)
}
