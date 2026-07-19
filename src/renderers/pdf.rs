//! mdbook-pdf 渲染器入口
//!
//! 实现 mdBook Renderer trait，负责配置解析、后端调度、后处理触发。
//!
//! # 工作流程
//!
//! 1. 从 stdin 读取 RenderContext
//! 2. 解析 book.toml 中的 [output.pdf] 配置
//! 3. 定位 print.html
//! 4. 提取章节路径和书籍元数据
//! 5. HTML 预处理
//! 6. 后端调度 (CDP → CLI 回退)
//! 7. PDF 后处理 (书签 + 元数据)
//! 8. 输出日志

use mdbook_core::book::{Book, BookItem};
use mdbook_renderer::{RenderContext, Renderer};

use super::pdf_chrome_cdp;
use super::pdf_chrome_cdp_light;
use super::pdf_html_preprocess;
use super::pdf_outline;

/// PDF 渲染器
pub struct PdfRenderer;

impl Renderer for PdfRenderer {
    fn name(&self) -> &str {
        "pdf"
    }

    fn render(&self, ctx: &RenderContext) -> Result<(), mdbook_core::errors::Error> {
        run_pdf(ctx).map_err(|e| mdbook_core::errors::Error::msg(format!("PDF 渲染失败: {}", e)))
    }
}

/// 运行 PDF 渲染流程
pub fn run_pdf(ctx: &RenderContext) -> Result<(), anyhow::Error> {
    // 1. 解析配置
    let cfg: PdfOptions = ctx
        .config
        .get::<toml::Value>("output.pdf")
        .ok()
        .flatten()
        .as_ref()
        .and_then(|v| PdfOptions::from_toml_value(v))
        .unwrap_or_default();

    // 2. 定位 print.html
    let html_dir = ctx
        .destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法获取目标目录的父目录"))?
        .join("html");
    let print_html_path = html_dir.join("print.html");

    if !print_html_path.exists() {
        log::warn!(
            "print.html 不存在于 {}. \
             请确认 output.html 已启用且 output.html.print.enable 设置为 true。",
            print_html_path.display()
        );
        return Ok(());
    }

    // 3. 提取章节路径
    let chapter_paths: Vec<String> = extract_chapter_paths(&ctx.book);

    // 4. 提取书籍元数据
    let book_title = ctx.config.book.title.as_deref();
    let book_authors = if ctx.config.book.authors.is_empty() {
        None
    } else {
        Some(ctx.config.book.authors.join(", "))
    };
    let book_language = ctx.config.book.language.as_deref();

    // 5. 读取 print.html 内容
    let html_content = std::fs::read_to_string(&print_html_path)?;

    // 6. HTML 预处理
    let processed_html = pdf_html_preprocess::preprocess(&html_content, &chapter_paths, &cfg);

    // 7. 确定输出路径
    let output_pdf = ctx.destination.join("output.pdf");

    // 8. 创建临时 HTML 文件
    let temp_html = html_dir.join("print_pdf_temp.html");

    // 9. 后端调度
    let backend_result = match cfg.backend.as_str() {
        "chrome-cli" => {
            log::info!("使用 Chrome CLI 后端");
            std::fs::write(&temp_html, &processed_html)?;
            pdf_chrome_cdp::render_chrome_cli(&temp_html, &output_pdf, &cfg)
        }
        "chrome-legacy" => {
            log::info!("使用 Chrome CDP 后端 (chromiumoxide)");
            let result =
                pdf_chrome_cdp::render_chrome_cdp(&processed_html, &output_pdf, &cfg, &temp_html);
            match result {
                Ok(()) => Ok(()),
                Err(e) => {
                    log::warn!("CDP 后端失败 ({}), 回退到 CLI 模式...", e);
                    std::fs::write(&temp_html, &processed_html)?;
                    pdf_chrome_cdp::render_chrome_cli(&temp_html, &output_pdf, &cfg)
                }
            }
        }
        _ => {
            log::info!("使用轻量 CDP 后端");
            let result = pdf_chrome_cdp_light::render_chrome_cdp_light(
                &processed_html, &output_pdf, &cfg, &temp_html,
            );
            match result {
                Ok(()) => Ok(()),
                Err(e) => {
                    log::warn!("轻量 CDP 失败 ({}), 回退到 CLI 模式...", e);
                    std::fs::write(&temp_html, &processed_html)?;
                    pdf_chrome_cdp::render_chrome_cli(&temp_html, &output_pdf, &cfg)
                }
            }
        }
    };

    // 处理后端结果
    if let Err(e) = &backend_result {
        log::error!("所有 PDF 生成后端均失败: {}", e);
        let _ = std::fs::remove_file(&temp_html);
        return Err(anyhow::anyhow!("PDF 生成失败: {}", e));
    }

    log::info!("PDF 原始文件已生成: {}", output_pdf.display());

    // 10. PDF 后处理（非致命）
    if cfg.generate_document_outline {
        log::info!("执行 PDF 后处理（书签 + 元数据）...");
        let author_str = book_authors.as_deref();
        pdf_outline::postprocess_pdf(
            &output_pdf,
            &html_content,
            book_title,
            author_str,
            book_language,
        )?;
        log::info!("PDF 后处理完成");
    }

    // 11. 清理临时文件
    let _ = std::fs::remove_file(&temp_html);

    // 12. 输出日志
    if let Ok(metadata) = std::fs::metadata(&output_pdf) {
        log::info!(
            "PDF 成功生成: {} ({} 字节)",
            output_pdf.display(),
            metadata.len()
        );
    }

    Ok(())
}

/// 从书籍中提取所有章节路径
fn extract_chapter_paths(book: &Book) -> Vec<String> {
    let mut paths = Vec::new();
    book.iter().for_each(|item| {
        if let BookItem::Chapter(ch) = item {
            if let Some(ref path) = ch.path {
                paths.push(path.to_string_lossy().to_string());
            }
        }
    });
    paths
}

/// 运行 mdbook-pdf 渲染器（main.rs 调用的入口函数）
pub fn run() -> anyhow::Result<()> {
    let renderer = PdfRenderer;
    crate::utils::run_renderer(&renderer)
}

// ═══════════════════════════════════════════════════════════════
// 配置系统
// ═══════════════════════════════════════════════════════════════

/// PDF 输出配置
///
/// 对应 book.toml 中 `[output.pdf]` 节的配置项。
/// 所有字段均有默认值，无需用户完整配置。
#[derive(Debug, Clone)]
pub struct PdfOptions {
    // ── 后端选择 ──
    /// 后端类型: "chrome" (CDP) 或 "chrome-cli"
    pub backend: String,
    /// Chrome/Chromium 可执行文件路径
    pub browser_binary_path: String,
    /// 重试次数
    pub trying_times: u64,
    /// 超时秒数
    pub timeout: u64,

    // ── 页面几何（英寸） ──
    pub paper_width: f64,
    pub paper_height: f64,
    pub landscape: bool,
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub scale: f64,
    pub prefer_css_page_size: bool,

    // ── 页眉/页脚 ──
    /// 强制禁用页眉/页脚
    pub no_header: Option<bool>,
    /// 启用页眉/页脚显示
    pub display_header_footer: bool,
    /// 使用 CDP 原生页眉/页脚模板
    pub use_native_header_footer: bool,
    /// 使用 CSS 注入页眉/页脚
    pub css_header_footer: bool,
    /// CSS 页眉高度（英寸）
    pub header_height: f64,
    /// CSS 页脚高度（英寸）
    pub footer_height: f64,
    /// 页眉 HTML 模板
    pub header_template: String,
    /// 页脚 HTML 模板
    pub footer_template: String,

    // ── 内容控制 ──
    pub print_background: bool,
    pub page_range: String,
    pub generate_document_outline: bool,
    pub generate_tagged_pdf: bool,

    // ── 链接修复 ──
    pub static_site_url: String,
}

impl PdfOptions {
    /// 从 toml::Value 解析配置
    fn from_toml_value(value: &toml::Value) -> Option<Self> {
        let default = Self::default();

        match value {
            toml::Value::Table(table) => {
                let mut opts = default.clone();
                if let Some(v) = table.get("backend").and_then(|v| v.as_str()) {
                    opts.backend = v.to_string();
                }
                if let Some(v) = table.get("browser-binary-path").and_then(|v| v.as_str()) {
                    opts.browser_binary_path = v.to_string();
                }
                if let Some(v) = table.get("trying-times").and_then(|v| v.as_integer()) {
                    opts.trying_times = v.max(1) as u64;
                }
                if let Some(v) = table.get("timeout").and_then(|v| v.as_integer()) {
                    opts.timeout = v.max(30) as u64;
                }
                if let Some(v) = table.get("paper-width").and_then(|v| v.as_float()) {
                    opts.paper_width = v;
                }
                if let Some(v) = table.get("paper-height").and_then(|v| v.as_float()) {
                    opts.paper_height = v;
                }
                if let Some(v) = table.get("landscape").and_then(|v| v.as_bool()) {
                    opts.landscape = v;
                }
                if let Some(v) = table.get("margin-top").and_then(|v| v.as_float()) {
                    opts.margin_top = v;
                }
                if let Some(v) = table.get("margin-bottom").and_then(|v| v.as_float()) {
                    opts.margin_bottom = v;
                }
                if let Some(v) = table.get("margin-left").and_then(|v| v.as_float()) {
                    opts.margin_left = v;
                }
                if let Some(v) = table.get("margin-right").and_then(|v| v.as_float()) {
                    opts.margin_right = v;
                }
                if let Some(v) = table.get("scale").and_then(|v| v.as_float()) {
                    opts.scale = v;
                }
                if let Some(v) = table.get("prefer-css-page-size").and_then(|v| v.as_bool()) {
                    opts.prefer_css_page_size = v;
                }
                if let Some(v) = table.get("no-header").and_then(|v| v.as_bool()) {
                    opts.no_header = Some(v);
                }
                if let Some(v) = table.get("display-header-footer").and_then(|v| v.as_bool()) {
                    opts.display_header_footer = v;
                }
                if let Some(v) = table.get("use-native-header-footer").and_then(|v| v.as_bool()) {
                    opts.use_native_header_footer = v;
                }
                if let Some(v) = table.get("css-header-footer").and_then(|v| v.as_bool()) {
                    opts.css_header_footer = v;
                }
                if let Some(v) = table.get("header-height").and_then(|v| v.as_float()) {
                    opts.header_height = v;
                }
                if let Some(v) = table.get("footer-height").and_then(|v| v.as_float()) {
                    opts.footer_height = v;
                }
                if let Some(v) = table.get("header-template").and_then(|v| v.as_str()) {
                    opts.header_template = v.to_string();
                }
                if let Some(v) = table.get("footer-template").and_then(|v| v.as_str()) {
                    opts.footer_template = v.to_string();
                }
                if let Some(v) = table.get("print-background").and_then(|v| v.as_bool()) {
                    opts.print_background = v;
                }
                if let Some(v) = table.get("page-range").and_then(|v| v.as_str()) {
                    opts.page_range = v.to_string();
                }
                if let Some(v) = table.get("generate-document-outline").and_then(|v| v.as_bool()) {
                    opts.generate_document_outline = v;
                }
                if let Some(v) = table.get("generate-tagged-pdf").and_then(|v| v.as_bool()) {
                    opts.generate_tagged_pdf = v;
                }
                if let Some(v) = table.get("static-site-url").and_then(|v| v.as_str()) {
                    opts.static_site_url = v.to_string();
                }

                Some(opts)
            }
            _ => Some(default),
        }
    }

    /// 判断页眉/页脚是否启用
    ///
    /// 遵循优先级: no-header = true → 强制禁用
    pub fn header_footer_enabled(&self) -> bool {
        match self.no_header {
            Some(true) => false,
            _ => self.display_header_footer,
        }
    }
}

impl Default for PdfOptions {
    fn default() -> Self {
        PdfOptions {
            backend: "chrome".to_string(),
            browser_binary_path: String::new(),
            trying_times: 1,
            timeout: 600,
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
            generate_document_outline: true,
            generate_tagged_pdf: true,
            static_site_url: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdf_options_default() {
        let cfg = PdfOptions::default();
        assert_eq!(cfg.backend, "chrome");
        assert_eq!(cfg.paper_width, 8.5);
        assert_eq!(cfg.paper_height, 11.0);
        assert_eq!(cfg.margin_top, 1.0);
        assert!(!cfg.landscape);
        assert!(!cfg.use_native_header_footer);
        assert!(cfg.css_header_footer);
        assert!(cfg.generate_document_outline);
    }

    #[test]
    fn test_header_footer_enabled_no_header_overrides() {
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        cfg.no_header = Some(true);
        assert!(!cfg.header_footer_enabled());
    }

    #[test]
    fn test_header_footer_enabled_default_off() {
        let cfg = PdfOptions::default();
        assert!(!cfg.header_footer_enabled());
    }

    #[test]
    fn test_header_footer_enabled_on() {
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        assert!(cfg.header_footer_enabled());
    }

    #[test]
    fn test_from_toml_value_basic() {
        let toml_str = r#"
backend = "chrome-cli"
paper-width = 8.0
paper-height = 10.0
landscape = true
print-background = false
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let cfg = PdfOptions::from_toml_value(&value).unwrap();
        assert_eq!(cfg.backend, "chrome-cli");
        assert!((cfg.paper_width - 8.0).abs() < f64::EPSILON);
        assert!(cfg.landscape);
        assert!(!cfg.print_background);
    }

    #[test]
    fn test_from_toml_value_empty() {
        let toml_str = "";
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        // 空 TOML 解析为空表，应返回 Some(默认值)
        let cfg = PdfOptions::from_toml_value(&value).unwrap();
        assert_eq!(cfg.backend, "chrome");
    }
}
