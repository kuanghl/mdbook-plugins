//! Chrome CDP 后端 — 通过 Chrome DevTools Protocol 生成 PDF
//!
//! 使用 `chromiumoxide` crate 连接 Chrome 实例，调用 `Page.printToPDF` 方法。

use std::path::Path;
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide_cdp::cdp::browser_protocol::page::{
    NavigateParams, PrintToPdfParams, PrintToPdfParamsBuilder,
};
use chromiumoxide_cdp::cdp::browser_protocol::target::CreateTargetParams;
use futures::StreamExt;
use url::Url;

use super::pdf::PdfOptions;

/// 通过 Chrome CDP 生成 PDF
///
/// 异步核心函数：启动浏览器 → 打开页面 → 调用 printToPDF → 返回 PDF 字节
pub async fn render_chrome_cdp_async(
    html_content: &str,
    output_pdf: &Path,
    cfg: &PdfOptions,
    temp_html_path: &Path,
) -> Result<(), anyhow::Error> {
    // 写入临时 HTML 文件
    std::fs::write(temp_html_path, html_content)?;

    // 构建浏览器配置 — 使用独立临时用户数据目录避免 SingletonLock 冲突
    let chrome_path = resolve_chrome_path(cfg);
    let user_data_dir = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("无法创建临时用户数据目录: {}", e))?;
    let data_dir_path = user_data_dir.path().to_path_buf();
    let mut builder = BrowserConfig::builder();
    if let Some(path) = chrome_path {
        builder = builder.chrome_executable(path);
    }
    let browser_config = builder
        .no_sandbox()
        .launch_timeout(Duration::from_secs(cfg.timeout))
        .request_timeout(Duration::from_secs(cfg.timeout))
        .user_data_dir(&data_dir_path)
        .build()
        .map_err(|e| anyhow::anyhow!("无法构建浏览器配置: {}", e))?;
    log::debug!("CDP BrowserConfig 超时: launch={}s, request={}s", cfg.timeout, cfg.timeout);

    // 启动浏览器
    let (mut browser, mut handler) = Browser::launch(browser_config)
        .await
        .map_err(|e| anyhow::anyhow!("无法启动 Chrome 浏览器: {}", e))?;
    // Handler 必须被驱动才能处理 CDP 消息
    tokio::spawn(async move {
        while let Some(_) = handler.next().await {}
    });

    let result = inner_render(&mut browser, temp_html_path, output_pdf, cfg).await;

    // 关闭浏览器
    let _ = browser.close().await;
    if let Ok(Some(child)) = browser.wait().await {
        let _ = child;
    }

    result
}

async fn inner_render(
    browser: &mut Browser,
    temp_html_path: &Path,
    output_pdf: &Path,
    cfg: &PdfOptions,
) -> Result<(), anyhow::Error> {
    // 构建 file:// URL
    let file_url = Url::from_file_path(temp_html_path)
        .map_err(|_| anyhow::anyhow!("无法将路径转换为 URL: {:?}", temp_html_path))?;
    log::info!("文件 URL: {}", file_url);

    // 创建新页面
    log::info!("正在创建新页面...");
    let page = browser
        .new_page(CreateTargetParams::new("about:blank"))
        .await
        .map_err(|e| anyhow::anyhow!("无法创建页面: {}", e))?;
    log::info!("页面已创建，正在导航到: {}", file_url);

    // 导航到文件 URL（带超时保护）
    log::info!("正在导航到: {}（超时 {}s）", file_url, cfg.timeout);
    let goto_result = tokio::time::timeout(
        Duration::from_secs(cfg.timeout),
        page.goto(NavigateParams::new(file_url.as_str())),
    )
    .await;

    match goto_result {
        Ok(Ok(_)) => log::info!("导航完成"),
        Ok(Err(e)) => return Err(anyhow::anyhow!("无法导航到 HTML 文件: {}", e)),
        Err(_) => return Err(anyhow::anyhow!("导航超时 ({}s)", cfg.timeout)),
    }

    // 等待内容加载哨兵元素（由 inject_js 注入）
    log::info!("正在等待内容加载...");
    let wait_result = tokio::time::timeout(
        Duration::from_secs(cfg.timeout),
        wait_for_content_loaded(&page),
    )
    .await;

    match wait_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            log::warn!("等待内容加载时出错 (将继续尝试生成 PDF): {}", e);
        }
        Err(_) => {
            log::warn!("等待内容加载超时 (将继续尝试生成 PDF)");
        }
    }

    // 构建 printToPDF 参数
    let params = build_print_to_pdf_params(cfg);
    log::info!("正在调用 page.pdf() 生成 PDF...");

    // 调用 Page.printToPDF
    let pdf_data = page
        .pdf(params)
        .await
        .map_err(|e| anyhow::anyhow!("CDP printToPDF 调用失败: {}", e))?;
    log::info!("PDF 数据已接收, {} 字节", pdf_data.len());

    // 写入输出文件
    std::fs::write(output_pdf, &pdf_data)?;

    log::info!(
        "PDF 已成功生成: {} ({} 字节)",
        output_pdf.display(),
        pdf_data.len()
    );
    Ok(())
}

/// 等待页面内容完全加载（通过检测哨兵元素）
async fn wait_for_content_loaded(page: &chromiumoxide::page::Page) -> Result<(), anyhow::Error> {
    for _ in 0..300 {
        if page
            .find_element("#content-has-all-loaded-for-mdbook-pdf-generation")
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow::anyhow!("内容加载哨兵元素未出现"))
}

/// 根据配置构建 CDP `PrintToPdfParams`
fn build_print_to_pdf_params(cfg: &PdfOptions) -> PrintToPdfParams {
    let hf_enabled = cfg.header_footer_enabled();
    let use_cdp_hf = hf_enabled && cfg.use_native_header_footer;

    let mut builder = PrintToPdfParamsBuilder::default();
    if cfg.landscape {
        builder = builder.landscape(true);
    }
    if use_cdp_hf {
        builder = builder.display_header_footer(true);
        if !cfg.header_template.is_empty() {
            builder = builder.header_template(cfg.header_template.clone());
        }
        if !cfg.footer_template.is_empty() {
            builder = builder.footer_template(cfg.footer_template.clone());
        }
    }
    if cfg.print_background {
        builder = builder.print_background(true);
    }
    if (cfg.scale - 1.0).abs() > f64::EPSILON {
        builder = builder.scale(cfg.scale);
    }
    builder = builder.paper_width(cfg.paper_width);
    builder = builder.paper_height(cfg.paper_height);
    builder = builder.margin_top(cfg.margin_top);
    builder = builder.margin_bottom(cfg.margin_bottom);
    builder = builder.margin_left(cfg.margin_left);
    builder = builder.margin_right(cfg.margin_right);
    if !cfg.page_range.is_empty() {
        builder = builder.page_ranges(cfg.page_range.clone());
    }
    if cfg.prefer_css_page_size {
        builder = builder.prefer_css_page_size(true);
    }
    if cfg.generate_tagged_pdf {
        builder = builder.generate_tagged_pdf(true);
    }
    // 文档大纲由后处理模块负责，不传 CDP
    builder = builder.generate_document_outline(false);

    builder.build()
}

/// 解析 Chrome 可执行文件路径
fn resolve_chrome_path(cfg: &PdfOptions) -> Option<std::path::PathBuf> {
    // 优先级 1: 环境变量 CHROME
    if let Ok(path) = std::env::var("CHROME") {
        let p = std::path::PathBuf::from(&path);
        if p.is_file() {
            return Some(p);
        }
    }

    // 优先级 2: 配置路径
    if !cfg.browser_binary_path.is_empty() {
        let p = std::path::PathBuf::from(&cfg.browser_binary_path);
        if p.is_file() {
            return Some(p);
        }
    }

    // 优先级 3: 让 chromiumoxide 自动检测
    None
}

/// CDP 渲染入口（带重试和超时）
pub fn render_chrome_cdp(
    html_content: &str,
    output_pdf: &Path,
    cfg: &PdfOptions,
    temp_html_path: &Path,
) -> Result<(), anyhow::Error> {
    let max_attempts = std::cmp::max(1, cfg.trying_times.max(1) as usize);

    for attempt in 1..=max_attempts {
        let rt = tokio::runtime::Runtime::new()?;
        match rt.block_on(async {
            render_chrome_cdp_async(html_content, output_pdf, cfg, temp_html_path).await
        }) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < max_attempts => {
                log::warn!(
                    "Chrome CDP 第 {}/{} 次尝试失败: {}. 500ms 后重试...",
                    attempt,
                    max_attempts,
                    e
                );
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Chrome CDP 在 {} 次尝试后均失败: {}",
                    max_attempts,
                    e
                ));
            }
        }
    }
    Ok(())
}

/// Chrome CLI 后端降级方案
///
/// 通过 `--headless --print-to-pdf` 命令行参数调用 Chrome
pub fn render_chrome_cli(
    print_html: &Path,
    output_pdf: &Path,
    cfg: &PdfOptions,
) -> Result<(), anyhow::Error> {
    let chrome = resolve_chrome_path(cfg)
        .or_else(find_chrome)
        .ok_or_else(|| anyhow::anyhow!("找不到 Chrome/Chromium 可执行文件"))?;

    let mut cmd = std::process::Command::new(&chrome);
    cmd.arg("--headless")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg(format!("--print-to-pdf={}", output_pdf.display()));

    // 使用独立临时用户数据目录避免 SingletonLock 冲突
    if let Ok(_tmp_dir) = std::fs::create_dir_all("/tmp/mdbook-pdf-runner") {
        let user_data_dir = format!("/tmp/mdbook-pdf-runner/{}", std::process::id());
        let _ = std::fs::remove_dir_all(&user_data_dir);
        cmd.arg(format!("--user-data-dir={}", user_data_dir));
    }

    if !cfg.header_footer_enabled() {
        cmd.arg("--print-to-pdf-no-header");
    }

    if cfg.landscape {
        cmd.arg("--landscape");
    }

    if cfg.header_footer_enabled() && cfg.use_native_header_footer {
        if !cfg.header_template.is_empty() {
            cmd.arg(format!("--header-template={}", cfg.header_template));
        }
        if !cfg.footer_template.is_empty() {
            cmd.arg(format!("--footer-template={}", cfg.footer_template));
        }
    }

    cmd.arg("--disable-software-rasterizer");
    cmd.arg("--disable-dev-shm-usage");
    cmd.arg(print_html.as_os_str());

    log::info!("执行 Chrome CLI: {:?}", cmd);

    // 设置超时，避免 Chrome 挂起导致构建卡住
    let timeout_secs = cfg.timeout.max(30);
    let output = wait_with_timeout(&mut cmd, timeout_secs)
        .map_err(|e| anyhow::anyhow!("Chrome CLI 执行失败或超时 ({}s): {}", timeout_secs, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "Chrome CLI 返回错误 (exit code: {}): {}",
            output.status,
            stderr.trim()
        ));
    }

    if !output_pdf.exists() {
        return Err(anyhow::anyhow!(
            "Chrome CLI 未生成 PDF 文件: {}",
            output_pdf.display()
        ));
    }

    let metadata = std::fs::metadata(output_pdf)?;
    log::info!(
        "PDF 已通过 CLI 生成: {} ({} 字节)",
        output_pdf.display(),
        metadata.len()
    );

    Ok(())
}

/// 通过平台默认路径查找 Chrome
fn find_chrome() -> Option<std::path::PathBuf> {
    let candidates: Vec<&str> = if cfg!(target_os = "linux") {
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
            "google-chrome",
            "chromium",
        ]
    } else {
        vec!["chrome", "chromium", "msedge"]
    };

    for name in &candidates {
        // 作为绝对路径检查
        let p = std::path::PathBuf::from(name);
        if p.is_file() {
            return Some(p);
        }
        // 在 PATH 中查找
        if let Some(path) = find_in_path(name) {
            return Some(path);
        }
    }

    None
}

/// 在 PATH 环境变量中查找可执行文件
fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 带超时的命令执行，避免 Chrome 挂起导致构建卡住
fn wait_with_timeout(
    cmd: &mut std::process::Command,
    timeout_secs: u64,
) -> Result<std::process::Output, anyhow::Error> {
    use std::sync::mpsc;
    use std::thread;

    // 启动子进程
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("无法启动 Chrome: {}", e))?;

    let child_pid = child.id();
    let (tx, rx) = mpsc::channel();

    // 在线程中等待子进程完成并收集输出
    thread::spawn(move || {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(ref mut out) = child.stdout {
            let _ = std::io::Read::read_to_end(out, &mut stdout);
        }
        if let Some(ref mut err) = child.stderr {
            let _ = std::io::Read::read_to_end(err, &mut stderr);
        }
        let status = child.wait();
        let output = std::process::Output {
            status: status.unwrap_or_default(),
            stdout,
            stderr,
        };
        let _ = tx.send(output);
    });

    // 等待结果（带超时）
    match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        Ok(output) => Ok(output),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // 超时：杀掉子进程
            unsafe {
                libc::kill(child_pid as i32, libc::SIGKILL);
            }
            Err(anyhow::anyhow!("Chrome 执行超时 ({}s)，已终止进程 (PID {})", timeout_secs, child_pid))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(anyhow::anyhow!("Chrome 进程意外断开"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pdf_params_default() {
        let cfg = PdfOptions::default();
        let params = build_print_to_pdf_params(&cfg);

        assert_eq!(params.landscape, None);
        assert_eq!(params.display_header_footer, None);
        // print_background 默认 true，所以为 Some(true)
        assert_eq!(params.print_background, Some(true));
        assert_eq!(params.scale, None);
        assert_eq!(params.paper_width, Some(8.5));
        assert_eq!(params.paper_height, Some(11.0));
        assert_eq!(params.generate_document_outline, Some(false));
    }

    #[test]
    fn test_build_pdf_params_landscape() {
        let mut cfg = PdfOptions::default();
        cfg.landscape = true;
        let params = build_print_to_pdf_params(&cfg);
        assert_eq!(params.landscape, Some(true));
    }

    #[test]
    fn test_build_pdf_params_native_header_footer() {
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        cfg.use_native_header_footer = true;
        cfg.header_template = "<span class='title'></span>".into();
        let params = build_print_to_pdf_params(&cfg);
        assert_eq!(params.display_header_footer, Some(true));
        assert_eq!(
            params.header_template,
            Some("<span class='title'></span>".to_string())
        );
    }
}
