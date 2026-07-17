//! mdbook-pdf — PDF 渲染器（轻量版）
//!
//! 依赖系统安装的 Chrome/Chromium，通过 `--headless --print-to-pdf` 命令行生成 PDF。
//! 零额外 Rust crate 依赖（仅使用标准库 + 已有依赖）。
//!
//! book.toml 配置：
//! ```toml
//! [output.pdf]
//! # browser-binary-path = "/usr/bin/chromium-browser"
//! # landscape = false
//! # paper-width = 8.5
//! # paper-height = 11.0
//! # margin-top = 1.0
//! # margin-bottom = 1.0
//! # margin-left = 1.0
//! # margin-right = 1.0
//! ```

use mdbook_renderer::{RenderContext, Renderer};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

// =========================================================================
// 配置结构体（从 book.toml 反序列化）
// =========================================================================

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(default, rename_all = "kebab-case")]
pub struct PdfOptions {
    pub browser_binary_path: String,
    pub landscape: bool,
    pub print_background: bool,
    pub paper_width: f64,
    pub paper_height: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub prefer_css_page_size: bool,
    pub scale: f64,
    pub no_header: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            browser_binary_path: String::new(),
            landscape: false,
            print_background: true,
            paper_width: 8.5,
            paper_height: 11.0,
            margin_top: 1.0,
            margin_bottom: 1.0,
            margin_left: 1.0,
            margin_right: 1.0,
            prefer_css_page_size: false,
            scale: 1.0,
            no_header: true,
        }
    }
}

pub struct PdfRenderer;

impl Renderer for PdfRenderer {
    fn name(&self) -> &str {
        "pdf"
    }

    fn render(&self, ctx: &RenderContext) -> Result<(), mdbook_core::errors::Error> {
        run_pdf(ctx)
    }
}

fn run_pdf(ctx: &RenderContext) -> Result<(), mdbook_core::errors::Error> {
    let cfg: Option<toml::Value> = ctx
        .config
        .get("output.pdf")
        .ok()
        .flatten();
    let cfg = cfg
        .map(|v| {
            // toml::Value → serde_json::Value
            let json_val = serde_json::to_value(v).unwrap_or_default();
            serde_json::from_value::<PdfOptions>(json_val).unwrap_or_default()
        })
        .unwrap_or_default();

    // 检查 Chrome 是否可用，不可用则静默跳过 PDF 生成
    if let Err(e) = resolve_chrome(&cfg.browser_binary_path) {
        log::debug!("mdbook-pdf: Chrome not available, skipping PDF generation: {}", e);
        return Ok(());
    }

    // HTML 后端必须在 PDF 之前运行，生成 print.html
    let html_dir = ctx
        .destination
        .parent()
        .unwrap_or(&ctx.destination)
        .join("html");
    let print_html = html_dir.join("print.html");

    if !print_html.exists() {
        log::warn!(
            "mdbook-pdf: print.html not found at {}. \
             Make sure [output.html] is enabled and [output.html.print] enable = true.",
            print_html.display()
        );
        return Ok(());
    }

    // 读取 print.html 并注入 @page CSS
    let mut html_content = std::fs::read_to_string(&print_html)?;
    html_content = inject_page_style(&html_content, &cfg);

    // 写入临时文件（添加 pdf 配置后的 HTML）
    let temp_html = html_dir.join("print_pdf.html");
    {
        let mut f = std::fs::File::create(&temp_html)?;
        f.write_all(html_content.as_bytes())?;
    }

    // 寻找 Chrome/Chromium 可执行文件
    let chrome = resolve_chrome(&cfg.browser_binary_path)
        .expect("Chrome availability already verified above");
    let output_pdf = ctx.destination.join("output.pdf");
    if let Some(parent) = output_pdf.parent() {
        std::fs::create_dir_all(parent)?;
    }

    log::info!(
        "mdbook-pdf: generating PDF with {}...",
        chrome.display()
    );
    log::info!("  input: {}", temp_html.display());
    log::info!("  output: {}", output_pdf.display());

    // 构建命令行参数
    let mut cmd = Command::new(&chrome);
    cmd.arg("--headless")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg(format!(
            "--print-to-pdf={}",
            output_pdf.to_string_lossy()
        ))
        .arg(format!(
            "--print-to-pdf-no-header={}",
            if cfg.no_header { "true" } else { "false" }
        ));

    // 纸张和边距通过 @page CSS 控制（已注入到 HTML），
    // 但 Chrome 命令行也支持基本设置
    if cfg.landscape {
        cmd.arg("--print-to-pdf=--landscape");
    }

    cmd.arg(temp_html.to_string_lossy().as_ref());

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("mdbook-pdf: failed to launch Chrome: {}. Skipping PDF.", e);
            return Ok(());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("mdbook-pdf: Chrome exited with error, skipping PDF:\n{}", stderr);
    }

    // 清理临时文件
    let _ = std::fs::remove_file(&temp_html);

    if output_pdf.exists() {
        let size = std::fs::metadata(&output_pdf)
            .map(|m| m.len())
            .unwrap_or(0);
        log::info!(
            "mdbook-pdf: PDF generated successfully ({:.1} KB)",
            size as f64 / 1024.0
        );
    }

    Ok(())
}

/// 注入 @page CSS 规则到 HTML 中
fn inject_page_style(html: &str, cfg: &PdfOptions) -> String {
    let page_css = format!(
        r#"@page {{
  size: {w}in {h}in{landscape};
  margin: {mt}in {mr}in {mb}in {ml}in;
}}
@media print {{
  body {{
    -webkit-print-color-adjust: exact;
    print-color-adjust: exact;
  }}
}}
"#,
        w = cfg.paper_width,
        h = cfg.paper_height,
        landscape = if cfg.landscape { " landscape" } else { "" },
        mt = cfg.margin_top,
        mr = cfg.margin_right,
        mb = cfg.margin_bottom,
        ml = cfg.margin_left,
    );

    let style_tag = format!("<style>\n{page_css}</style>\n");
    if let Some(pos) = html.find("</head>") {
        let mut result = String::with_capacity(html.len() + style_tag.len());
        result.push_str(&html[..pos]);
        result.push_str(&style_tag);
        result.push_str(&html[pos..]);
        result
    } else {
        // 没有 </head>，直接追加到开头
        format!("{style_tag}\n{html}")
    }
}

/// 查找 Chrome/Chromium 可执行文件
fn resolve_chrome(browser_path: &str) -> Result<PathBuf, mdbook_core::errors::Error> {
    // 1. 环境变量 CHROME
    if let Ok(path) = std::env::var("CHROME") {
        let p = PathBuf::from(&path);
        if p.is_file() {
            return Ok(p);
        }
    }

    // 2. 用户配置路径
    if !browser_path.is_empty() {
        let p = PathBuf::from(browser_path);
        if p.is_file() {
            return Ok(p);
        }
        log::warn!(
            "mdbook-pdf: configured browser-path '{}' not found, trying auto-detect",
            browser_path
        );
    }

    // 3. 自动检测
    let candidates = if cfg!(target_os = "linux") {
        vec![
            "google-chrome-stable",
            "google-chrome",
            "chromium-browser",
            "chromium",
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        ]
    } else {
        vec!["google-chrome-stable", "chromium-browser"]
    };

    for name in &candidates {
        let p = Path::new(name);
        if p.is_file() || which(name).is_some() {
            return Ok(p.to_path_buf());
        }
    }

    Err(mdbook_core::errors::Error::msg(
        "Chrome/Chromium not found. Install chromium, set CHROME env var, \
         or configure browser-binary-path in [output.pdf].",
    ))
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|d| {
            let f = d.join(name);
            if f.is_file() {
                Some(f)
            } else {
                None
            }
        })
    })
}

/// 运行 mdbook-pdf 渲染器
pub fn run() -> anyhow::Result<()> {
    crate::utils::run_renderer(&PdfRenderer)
}
