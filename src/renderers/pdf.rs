//! mdbook-pdf — PDF 渲染器（双后端：Chrome CDP / CLI）
//!
//! 支持两种 PDF 生成后端（通过 `backend` 配置切换）：
//! - `chrome`   （默认）通过 Chrome CDP 协议或 CLI `--headless --print-to-pdf` 生成
//! - `chrome-cli` 强制使用 CLI 模式（降级用）
//!
//! 页眉/页脚支持两种模式（`use-native-header-footer`）：
//! - `false`（默认）使用 position:fixed CSS 注入 + @page 边距补偿，兼容所有 Chrome 版本
//! - `true`  使用 CDP 原生 headerTemplate/footerTemplate，需要 Chrome 125+ 获得最佳效果
//!
//! book.toml 配置示例：
//! ```toml
//! [output.pdf]
//! command = "mdbook-plugins pdf"
//! backend = "chrome"
//!
//! # Chrome 路径（留空自动探测，支持环境变量 CHROME）
//! # browser-binary-path = "/usr/bin/chromium-browser"
//!
//! # 页面几何（所有单位均为英寸）
//! landscape = false
//! paper-width = 8.5
//! paper-height = 11.0
//! margin-top = 1.0
//! margin-bottom = 1.0
//! margin-left = 1.0
//! margin-right = 1.0
//! scale = 1.0
//!
//! # 页眉/页脚（no_header=true 优先覆盖 display-header-footer）
//! display-header-footer = false
//! use-native-header-footer = false
//! header-height = 0.7          # 固定定位模式：页眉高度（英寸）
//! footer-height = 0.6          # 固定定位模式：页脚高度（英寸）
//! header-template = ""
//! footer-template = ""
//!
//! # 内容控制
//! print-background = true
//! page-range = ""
//! ignore-invalid-page-ranges = false
//! generate-document-outline = true
//! generate-tagged-pdf = true
//! trying-times = 1
//!
//! # 链接修复
//! # static-site-url = "https://example.com/book"
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
    // ── 后端选择 ──
    /// PDF 生成后端："chrome"（默认，CDP→CLI 回退）或 "chrome-cli"（强制 CLI）
    pub backend: String,
    /// Chrome/Chromium 可执行文件路径（仅 chrome 后端使用），支持环境变量 CHROME
    pub browser_binary_path: String,
    /// PDF 生成失败时的最大重试次数（仅 CDP 模式）
    pub trying_times: u32,

    // ── 页面几何（单位：英寸） ──
    pub paper_width: f64,
    pub paper_height: f64,
    pub landscape: bool,
    /// 上边距（英寸）。默认 1.0 英寸 ≈ 2.54cm。
    pub margin_top: f64,
    /// 下边距（英寸）
    pub margin_bottom: f64,
    /// 左边距（英寸）
    pub margin_left: f64,
    /// 右边距（英寸）
    pub margin_right: f64,
    /// 全局缩放因子，1.25 即放大 125%
    pub scale: f64,
    /// 是否以 CSS @page 尺寸为准
    pub prefer_css_page_size: bool,

    // ── 页眉/页脚 ──
    /// 设为 true 则无条件跳过页眉页脚渲染（优先覆盖 display_header_footer）。
    /// 设为 false 或未设置时以 display_header_footer 为准。
    #[serde(default)]
    pub no_header: Option<bool>,
    /// 是否启用页眉/页脚（被 no_header=true 覆盖）
    pub display_header_footer: bool,
    /// true → 使用 CDP 原生 headerTemplate/footerTemplate（推荐 Chrome 125+）
    /// false → 使用 position:fixed CSS 注入 + @page 边距补偿（兼容所有版本）
    pub use_native_header_footer: bool,

    // ── CSS 注入独立开关 ──
    /// 是否启用 CSS 注入方式的页眉/页脚。
    /// - true（默认）：注入 CSS 页眉/页脚（与 CDP 原生可共存）
    /// - false：不注入 CSS 页眉/页脚
    ///
    /// 与 use_native_header_footer 组合可实现 4 种模式：
    /// 1. use_native_header_footer=false + css_header_footer=true  → 仅 CSS 注入（默认）
    /// 2. use_native_header_footer=false + css_header_footer=false → 无页眉/页脚
    /// 3. use_native_header_footer=true  + css_header_footer=true  → CDP 原生 + CSS 注入（双页眉/页脚）
    /// 4. use_native_header_footer=true  + css_header_footer=false → 仅 CDP 原生
    pub css_header_footer: bool,
    /// 固定定位模式下，页眉占用的英寸高度（用于 @page margin-top 补偿）
    pub header_height: f64,
    /// 固定定位模式下，页脚占用的英寸高度（用于 @page margin-bottom 补偿）
    pub footer_height: f64,
    /// 页眉 HTML 模板（原生模式支持 class="date/title/pageNumber/totalPages"）
    #[serde(default)]
    pub header_template: String,
    /// 页脚 HTML 模板
    #[serde(default)]
    pub footer_template: String,

    // ── 内容控制 ──
    /// 是否打印背景色/背景图
    pub print_background: bool,
    /// 页码范围，如 "1-5,8,11-13"，空字符串表示全部
    #[serde(default)]
    pub page_range: String,
    /// true → 页码范围无效时忽略并生成全部；false → 报错
    pub ignore_invalid_page_ranges: bool,
    /// 是否生成 PDF 书签大纲
    pub generate_document_outline: bool,
    /// 是否生成带标签的 PDF（无障碍支持）
    pub generate_tagged_pdf: bool,

    // ── 链接修复 ──
    /// 书籍的静态网站基准 URL，用于将相对链接转为绝对路径
    #[serde(default)]
    pub static_site_url: String,
}

impl PdfOptions {
    /// 页眉/页脚是否实际生效。
    ///
    /// - `no_header = Some(true)` 时始终返回 `false`（向后兼容覆写）。
    /// - 其他情况（未设置或 `Some(false)`）以 `display_header_footer` 为准。
    pub fn header_footer_enabled(&self) -> bool {
        match self.no_header {
            Some(true) => false,
            _ => self.display_header_footer,
        }
    }
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            backend: "chrome".to_string(),
            browser_binary_path: String::new(),
            trying_times: 1,
            paper_width: 8.5,
            paper_height: 11.0,
            landscape: false,
            margin_top: 1.0,
            margin_bottom: 1.0,
            margin_left: 1.0,
            margin_right: 1.0,
            scale: 1.0,
            prefer_css_page_size: false,
            no_header: None,
            display_header_footer: false,
            use_native_header_footer: false,
            css_header_footer: true,
            header_height: 0.7,
            footer_height: 0.6,
            header_template: String::new(),
            footer_template: String::new(),
            print_background: true,
            page_range: String::new(),
            ignore_invalid_page_ranges: false,
            generate_document_outline: true,
            generate_tagged_pdf: true,
            static_site_url: String::new(),
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
    use mdbook_core::book::BookItem;

    let cfg: Option<toml::Value> = ctx
        .config
        .get("output.pdf")
        .ok()
        .flatten();
    let cfg = cfg
        .map(|v| {
            let json_val = serde_json::to_value(v).unwrap_or_default();
            serde_json::from_value::<PdfOptions>(json_val).unwrap_or_default()
        })
        .unwrap_or_default();

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

    let output_pdf = ctx.destination.join("output.pdf");

    // 提取章节路径（用于 ToC 修复和书签生成）
    let chapter_paths: Vec<String> = ctx
        .book
        .iter()
        .filter_map(|item| {
            if let BookItem::Chapter(ch) = item {
                ch.path.as_ref().map(|p| p.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();

    // 提取书籍元数据
    let book_cfg = &ctx.config.book;
    let book_title = book_cfg.title.as_deref().unwrap_or("");
    let book_authors: Vec<String> = book_cfg.authors.clone();
    let book_description = book_cfg.description.as_deref().unwrap_or("");
    let book_language = book_cfg.language.as_deref().unwrap_or("");

    // =====================================================================
    // 后端调度
    // =====================================================================
    match cfg.backend.as_str() {
        "chrome-cli" => {
            // 强制使用 CLI 模式（测试/降级用）
            render_chrome(&print_html, &output_pdf, &cfg, &html_dir)?;
        }
        _ => {
            // 默认 "chrome"：尝试 CDP，失败时回退到 CLI
            let html_content = std::fs::read_to_string(&print_html)?;
            let book_meta = super::pdf_chrome_cdp::BookMeta {
                title: book_title,
                authors: &book_authors,
                description: book_description,
                language: book_language,
            };
            match super::pdf_chrome_cdp::render_chrome_cdp(
                &html_content,
                &output_pdf,
                &print_html,
                &html_dir,
                &cfg,
                &chapter_paths,
                book_meta,
            ) {
                Ok(()) => {}
                Err(e) => {
                    log::warn!(
                        "mdbook-pdf(chrome-cdp): CDP failed ({}). \
                         Falling back to CLI mode. \
                         Try setting RUST_LOG=mdbook_plugins=debug for details.",
                        e
                    );
                    log::debug!("mdbook-pdf(chrome-cdp): cfg={:?}", cfg);
                    render_chrome(&print_html, &output_pdf, &cfg, &html_dir)?;
                }
            }
        }
    }

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

/// 使用 Chrome/Chromium headless 生成 PDF
fn render_chrome(
    print_html: &Path,
    output_pdf: &Path,
    cfg: &PdfOptions,
    html_dir: &Path,
) -> Result<(), mdbook_core::errors::Error> {
    // 检查 Chrome 是否可用
    if let Err(e) = resolve_chrome(&cfg.browser_binary_path) {
        log::debug!("mdbook-pdf(chrome): Chrome not available, skipping: {}", e);
        return Ok(());
    }

    // 读取 print.html 并注入 @page CSS
    let mut html_content = std::fs::read_to_string(print_html)?;
    html_content = inject_page_style(&html_content, cfg);

    // 写入临时文件
    let temp_html = html_dir.join("print_pdf.html");
    {
        let mut f = std::fs::File::create(&temp_html)?;
        f.write_all(html_content.as_bytes())?;
    }

    // 查找 Chrome/Chromium
    let chrome = resolve_chrome(&cfg.browser_binary_path)
        .expect("Chrome availability already verified above");

    if let Some(parent) = output_pdf.parent() {
        std::fs::create_dir_all(parent)?;
    }

    log::info!(
        "mdbook-pdf(chrome): generating PDF with {}...",
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
            if cfg.header_footer_enabled() && cfg.use_native_header_footer { "false" } else { "true" }
        ));

    // 用户自定义页眉/页脚模板（仅当原生模式启用时传递）
    if cfg.header_footer_enabled() && cfg.use_native_header_footer {
        if !cfg.header_template.is_empty() {
            cmd.arg(format!("--header-template={}", cfg.header_template));
        }
        if !cfg.footer_template.is_empty() {
            cmd.arg(format!("--footer-template={}", cfg.footer_template));
        }
    }

    if cfg.landscape {
        cmd.arg("--landscape");
    }

    cmd.arg(temp_html.to_string_lossy().as_ref());

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("mdbook-pdf(chrome): failed to launch Chrome: {}. Skipping.", e);
            return Ok(());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("mdbook-pdf(chrome): Chrome exited with error:\n{}", stderr);
    }

    // 清理临时文件
    let _ = std::fs::remove_file(&temp_html);

    Ok(())
}

/// 注入 @page CSS 规则到 HTML 中（CLI 后端用）
///
/// CLI 模式使用 `--print-to-pdf-no-header` 控制 Chrome 原生页眉/页脚，
/// 无需手动注入固定元素，@page 边距直接使用用户配置值。
/// 注入的 CSS 规则与 CDP 模式的 `pdf_html_preprocess` 一致。
fn inject_page_style(html: &str, cfg: &PdfOptions) -> String {
    let page_css = format!(
        r#"@page {{
  size: {w}in {h}in{landscape};
  margin: {mt}in {mr}in {mb}in {ml}in;
}}
/* mdbook-pdf: 打印分页控制 */
@media print {{
  body {{
    -webkit-print-color-adjust: exact;
    print-color-adjust: exact;
  }}
  /* 标题后不紧跟分页 */
  h1, h2, h3, h4, h5, h6 {{ page-break-after: avoid; }}
  /* h1 章节始终从新页开始 */
  h1 {{ page-break-before: always; }}
  h2, h3 {{ page-break-before: avoid; }}
  /* 保护代码块、表格、图片不断页 */
  pre, code, table, figure, img, svg {{
    page-break-inside: avoid;
  }}
  /* 动态图表保护 */
  .mermaid, .echarts {{
    page-break-inside: avoid;
  }}
  /* 孤行/寡行控制 */
  p, li {{ widows: 2; orphans: 2; }}
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
