//! mdbook-pdf — Chrome CDP 后端
//!
//! 通过 Chrome DevTools Protocol (`Page.printToPDF`) 生成 PDF。
//!
//! 三种平行页眉/页脚模式（通过配置组合）：
//! - CDP 原生：由 `use-native-header-footer = true` 启用，通过 CDP headerTemplate 渲染
//! - CSS 注入：由 `css-header-footer = true`（默认）启用，通过 JS position:fixed 注入
//! - 总开关：`no-header = true` 强制禁用所有；`display-header-footer = false` 禁用
//!
//! 四种组合模式：
//! 1. `use-native-header-footer=false` + `css-header-footer=true`  → 仅 CSS 注入（默认）
//! 2. `use-native-header-footer=false` + `css-header-footer=false` → 无页眉/页脚
//! 3. `use-native-header-footer=true`  + `css-header-footer=true`  → CDP 原生 + CSS 注入（双页眉/页脚）
//! 4. `use-native-header-footer=true`  + `css-header-footer=false` → 仅 CDP 原生

use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use chromiumoxide_cdp::cdp::browser_protocol::page::PrintToPdfParams;
use futures::StreamExt;

use super::pdf::PdfOptions;
use super::pdf_html_preprocess;
use super::pdf_outline;

/// 书籍元数据（用于 PDF 后处理）
pub struct BookMeta<'a> {
    pub title: &'a str,
    pub authors: &'a [String],
    pub description: &'a str,
    pub language: &'a str,
}

// ── 公开接口 ──

/// 使用 Chrome CDP 生成 PDF（含重试机制）
pub fn render_chrome_cdp(
    html_content: &str,
    output_pdf: &Path,
    print_html_path: &Path,
    html_dir: &Path,
    cfg: &PdfOptions,
    chapter_paths: &[String],
    book_meta: BookMeta,
) -> Result<(), anyhow::Error> {
    let rt = get_tokio_runtime();
    let max_attempts = std::cmp::max(1, cfg.trying_times) as usize;

    let processed = pdf_html_preprocess::preprocess_html(html_content, cfg, chapter_paths);

    // 调试信息
    let _ = std::fs::write("/tmp/mdbook_debug.txt", format!(
        "html_dir: {:?}\ntemp_html: {:?}\nhtml_dir.exists: {}\ndestination: {:?}\n",
        html_dir,
        html_dir.join("print_pdf.html"),
        html_dir.exists(),
        output_pdf,
    ));

    let temp_html = html_dir.join("print_pdf.html");
    {
        let mut f = std::fs::File::create(&temp_html)?;
        f.write_all(processed.html.as_bytes())?;
    }

    // 判断是否启用 CSS 注入模式
    // css_header_footer 是完全独立的开关，与 use_native_header_footer 正交：
    // - true  → 注入 CSS 页眉/页脚（与 CDP 原生可共存，产生双页眉/页脚）
    // - false → 不注入 CSS 页眉/页脚
    let use_css = cfg.header_footer_enabled() && cfg.css_header_footer;

    let result = (|| -> Result<(), anyhow::Error> {
        for attempt in 1..=max_attempts {
            log::info!(
                "mdbook-pdf(chrome-cdp): attempt {}/{} (hf={}, native={}, css={})",
                attempt, max_attempts,
                cfg.header_footer_enabled(), cfg.use_native_header_footer, use_css,
            );

            match rt.block_on(async {
                render_chrome_cdp_async(&temp_html, output_pdf, cfg, use_css).await
            }) {
                Ok(()) => return Ok(()),
                Err(e) if attempt < max_attempts => {
                    log::warn!("mdbook-pdf: attempt {} failed: {}. Retrying...", attempt, e);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    })();

    let _ = std::fs::remove_file(&temp_html);

    if result.is_ok() && output_pdf.exists() {
        if let Err(e) = pdf_outline::postprocess_pdf(
            output_pdf, print_html_path, cfg,
            book_meta.title, book_meta.authors, book_meta.description, book_meta.language,
        ) {
            log::warn!("mdbook-pdf(postprocess): non-fatal error: {}", e);
        }
    }

    result
}

// ── 异步渲染核心 ──

async fn render_chrome_cdp_async(
    temp_html: &Path,
    output_pdf: &Path,
    cfg: &PdfOptions,
    use_css: bool,
) -> Result<(), anyhow::Error> {
    use chromiumoxide::browser::{Browser, BrowserConfig};

    let mut browser_builder = BrowserConfig::builder();
    if let Some(ref path) = resolve_chrome_path(&cfg.browser_binary_path) {
        browser_builder = browser_builder.chrome_executable(path);
    }
    let browser_config = browser_builder.no_sandbox().build()
        .map_err(|e| anyhow::anyhow!("BrowserConfig build failed: {}", e))?;

    let (mut browser, mut handler) = Browser::launch(browser_config).await?;
    let handler_handle = tokio::spawn(async move {
        while let Some(h) = handler.next().await { if h.is_err() { break; } }
    });

    let file_url = url::Url::from_file_path(temp_html)
        .map_err(|_| anyhow::anyhow!("failed file URL: {}", temp_html.display()))?;
    let page = browser.new_page(file_url.as_str()).await?;
    page.wait_for_navigation().await?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // CSS 注入模式：通过 JS 注入 position:fixed 页眉/页脚
    if use_css {
        let js = build_css_injection_js(cfg);
        if !js.is_empty() {
            page.evaluate(js.as_str()).await?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    let params = build_pdf_params(cfg);
    let pdf_data = page.pdf(params).await?;

    if pdf_data.is_empty() {
        return Err(anyhow::anyhow!("PDF generation returned empty data"));
    }

    browser.close().await?;
    handler_handle.await.ok();

    if let Some(parent) = output_pdf.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_pdf, &pdf_data)?;

    log::info!("mdbook-pdf: PDF generated ({} bytes)", pdf_data.len());
    Ok(())
}

// ── CSS 注入 JS ──

/// 构建 CSS 注入模式的 JS 代码
///
/// 修复：正则 `[^>]*?` 支持 class 前有其他属性（如 style）
fn build_css_injection_js(cfg: &PdfOptions) -> String {
    let has_header = !cfg.header_template.is_empty();
    let has_footer = !cfg.footer_template.is_empty();
    if !has_header && !has_footer { return String::new(); }

    let hh = if has_header { cfg.header_height } else { 0.0 };
    let fh = if has_footer { cfg.footer_height } else { 0.0 };
    let mt = cfg.margin_top + hh;
    let mb = cfg.margin_bottom + fh;

    let esc = |s: &str| -> String {
        s.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")
    };
    let eh = esc(&cfg.header_template);
    let ef = esc(&cfg.footer_template);

    format!(
        r#"(function() {{
  const now = new Date().toISOString().split('T')[0];
  const bookTitle = document.title || '';
  function fill(t) {{
    return t.replace(/\{{{{date}}\}}/g, now).replace(/\{{{{title}}\}}/g, bookTitle)
      .replace(/<span[^>]*?\bclass=(['""])date\1[^>]*?><\/span>/gi, now)
      .replace(/<span[^>]*?\bclass=(['""])title\1[^>]*?><\/span>/gi, bookTitle);
  }}
  const s = document.createElement('style');
  s.textContent = `
.pf-h,.pf-f {{ display:none; }}
@media print {{
  .pf-h,.pf-f {{ display:block; position:fixed; left:0; right:0; width:100%; overflow:hidden; box-sizing:border-box; z-index:10000; background:#fff; }}
  .pf-h {{ top:0; height:{hh}in; }}
  .pf-f {{ bottom:0; height:{fh}in; }}
}}
@page {{ margin:{mt}in {mr}in {mb}in {ml}in; }}
`; document.head.appendChild(s);
  if ({hh_js}) {{ const d = document.createElement('div'); d.className='pf-h'; d.innerHTML=fill(`{eh}`); document.body.appendChild(d); }}
  if ({fh_js}) {{ const d = document.createElement('div'); d.className='pf-f'; d.innerHTML=fill(`{ef}`); document.body.appendChild(d); }}
}})();
"#,
        hh=hh, fh=fh, mt=mt, mr=cfg.margin_right, mb=mb, ml=cfg.margin_left,
        hh_js=if has_header{"true"}else{"false"},
        fh_js=if has_footer{"true"}else{"false"},
        eh=eh, ef=ef,
    )
}

// ── CDP 参数构造 ──

/// 构建 CDP Page.printToPDF 参数
///
/// 两种模式可独立启用/禁用：
/// - CDP 原生模式：`use_native_header_footer = true` 时，通过 CDP 传入模板
/// - CSS 注入模式：由 `css-header-footer` 独立控制，与此处无关
/// - 两者同时启用时，CDP 原生和 CSS 注入页眉/页脚会叠加显示
fn build_pdf_params(cfg: &PdfOptions) -> PrintToPdfParams {
    let hf_enabled = cfg.header_footer_enabled();
    // CDP 原生模式：仅当 use_native_header_footer=true 时启用
    let use_cdp_hf = hf_enabled && cfg.use_native_header_footer;

    PrintToPdfParams {
        display_header_footer: if use_cdp_hf { Some(true) } else { None },
        header_template: if use_cdp_hf && !cfg.header_template.is_empty() {
            Some(cfg.header_template.clone())
        } else { None },
        footer_template: if use_cdp_hf && !cfg.footer_template.is_empty() {
            Some(cfg.footer_template.clone())
        } else { None },
        landscape: if cfg.landscape { Some(true) } else { None },
        print_background: Some(cfg.print_background),
        scale: scaled_or_none(cfg.scale),
        paper_width: Some(cfg.paper_width),
        paper_height: Some(cfg.paper_height),
        margin_top: Some(cfg.margin_top),
        margin_bottom: Some(cfg.margin_bottom),
        margin_left: Some(cfg.margin_left),
        margin_right: Some(cfg.margin_right),
        page_ranges: if cfg.page_range.is_empty() { None } else { Some(cfg.page_range.clone()) },
        prefer_css_page_size: Some(cfg.prefer_css_page_size),
        transfer_mode: None,
        generate_tagged_pdf: Some(cfg.generate_tagged_pdf),
        generate_document_outline: Some(cfg.generate_document_outline),
    }
}

fn scaled_or_none(scale: f64) -> Option<f64> {
    if (scale - 1.0).abs() > f64::EPSILON { Some(scale) } else { None }
}

// ── 辅助 ──

fn get_tokio_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}

fn resolve_chrome_path(browser_path: &str) -> Option<String> {
    if let Ok(path) = std::env::var("CHROME") {
        if std::path::Path::new(&path).is_file() { return Some(path); }
    }
    if !browser_path.is_empty() {
        if std::path::Path::new(browser_path).is_file() { return Some(browser_path.to_string()); }
        log::warn!("mdbook-pdf: browser-path '{}' not found, auto-detecting", browser_path);
    }
    let candidates = if cfg!(target_os = "linux") {
        vec!["google-chrome-stable","google-chrome","chromium-browser","chromium"]
    } else if cfg!(target_os = "macos") {
        vec!["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
             "/Applications/Chromium.app/Contents/MacOS/Chromium"]
    } else if cfg!(target_os = "windows") {
        vec!["C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
             "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe"]
    } else { vec!["google-chrome-stable","chromium-browser"] };
    for name in &candidates {
        let p = std::path::Path::new(name);
        if p.is_file() { return Some(p.to_string_lossy().to_string()); }
        if let Some(found) = which(name) { return Some(found.to_string_lossy().to_string()); }
    }
    None
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|d| {
            let f = d.join(name); if f.is_file() { Some(f) } else { None }
        })
    })
}

// ── 单元测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_css_mode() {
        // 默认 use_native_header_footer=false → CSS 注入模式
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        cfg.header_template = "<span>h</span>".to_string();
        let params = build_pdf_params(&cfg);
        assert_eq!(params.display_header_footer, None, "默认不应启用 CDP 原生");
    }

    #[test]
    fn test_cdp_native_mode() {
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        cfg.use_native_header_footer = true;
        cfg.header_template = "<span class='title'></span>".to_string();
        let params = build_pdf_params(&cfg);
        assert_eq!(params.display_header_footer, Some(true));
        assert!(params.header_template.is_some());
    }

    #[test]
    fn test_css_injection_js_generated() {
        let mut cfg = PdfOptions::default();
        cfg.header_template = "<span>h</span>".to_string();
        cfg.footer_template = "<span>f</span>".to_string();
        cfg.margin_top = 0.5;
        cfg.margin_bottom = 1.0;
        let js = build_css_injection_js(&cfg);
        assert!(js.contains("pf-h"));
        assert!(js.contains("pf-f"));
        assert!(js.contains("true"));
    }

    #[test]
    fn test_css_injection_empty_template() {
        let cfg = PdfOptions::default();
        assert!(build_css_injection_js(&cfg).is_empty());
    }

    #[test]
    fn test_no_header_overrides() {
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        cfg.no_header = Some(true);
        assert!(!cfg.header_footer_enabled());
    }

    #[test]
    fn test_scaled_or_none() {
        assert!(scaled_or_none(1.0).is_none());
        assert!(scaled_or_none(1.25).is_some());
    }
}
