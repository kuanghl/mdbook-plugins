//! 轻量级 CDP 客户端 — 直接通过 WebSocket 通信，替代 chromiumoxide
//!
//! 设计原则：
//! - 只实现 PDF 生成需要的 CDP 命令（Page.enable, Page.navigate, Page.printToPDF）
//! - 完全控制超时，不受第三方库限制
//! - 单文件实现，约 300 行

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::process::{Child, Command as TokioCommand};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::pdf::PdfOptions;

// ═══════════════════════════════════════════════════════════
// 公开接口
// ═══════════════════════════════════════════════════════════

/// 通过轻量 CDP 客户端渲染 PDF（同步接口，内部使用 block_on）
pub fn render_chrome_cdp_light(
    html_content: &str,
    output_pdf: &Path,
    cfg: &PdfOptions,
    temp_html_path: &Path,
) -> Result<()> {
    let mut last_err = None;
    let max_attempts = std::cmp::max(1, cfg.trying_times.max(1) as usize);

    for attempt in 1..=max_attempts {
        let rt = tokio::runtime::Runtime::new()?;
        match rt.block_on(render_chrome_cdp_light_async(
            html_content, output_pdf, cfg, temp_html_path,
        )) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < max_attempts => {
                log::warn!(
                    "轻量 CDP 第 {}/{} 次尝试失败: {}. 500ms 后重试...",
                    attempt, max_attempts, e
                );
                std::thread::sleep(Duration::from_millis(500));
                last_err = Some(e);
            }
            Err(e) => {
                last_err = Some(e);
                break;
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("未知错误")))
}

/// 异步渲染核心
async fn render_chrome_cdp_light_async(
    html_content: &str,
    output_pdf: &Path,
    cfg: &PdfOptions,
    temp_html_path: &Path,
) -> Result<()> {
    // 写入临时 HTML 文件
    std::fs::write(temp_html_path, html_content)?;

    let chrome_path = resolve_chrome_path(cfg);
    let timeout = Duration::from_secs(cfg.timeout);

    // 1. 启动 Chrome
    log::info!("轻量 CDP: 启动 Chrome...");
    let mut chrome = ChromeProcess::launch(chrome_path.as_deref(), timeout).await?;
    let ws_url = chrome.ws_url.clone();

    // 2. 连接 CDP
    log::info!("轻量 CDP: 连接 WebSocket...");
    let mut cdp = CdpSession::connect(&ws_url, timeout).await?;

    // 使用 Result 来统一处理错误和清理
    let result = render_inner(&mut cdp, temp_html_path, output_pdf, cfg, timeout).await;

    // 3. 关闭 Chrome（无论成功失败）
    let _ = chrome.kill().await;

    result
}

/// 构建 file:// URL
fn file_url(path: &Path) -> Result<String> {
    let url = url::Url::from_file_path(path)
        .map_err(|_| anyhow::anyhow!("无法将路径转换为 URL: {:?}", path))?;
    Ok(url.to_string())
}

/// 查找 Chrome 可执行文件路径
fn resolve_chrome_path(cfg: &PdfOptions) -> Option<std::path::PathBuf> {
    // 环境变量 CHROME 优先
    if let Ok(path) = std::env::var("CHROME") {
        let p = std::path::PathBuf::from(&path);
        if p.is_file() {
            return Some(p);
        }
    }
    // 配置路径
    if !cfg.browser_binary_path.is_empty() {
        let p = std::path::PathBuf::from(&cfg.browser_binary_path);
        if p.is_file() {
            return Some(p);
        }
    }
    // 自动检测
    find_chrome_in_path()
}

fn find_chrome_in_path() -> Option<std::path::PathBuf> {
    let candidates = if cfg!(target_os = "linux") {
        vec!["google-chrome-stable", "google-chrome", "chromium-browser", "chromium"]
    } else if cfg!(target_os = "macos") {
        vec!["google-chrome", "chromium"]
    } else {
        vec!["chrome", "chromium", "msedge"]
    };
    for name in &candidates {
        if let Some(path) = search_path(name) {
            return Some(path);
        }
    }
    None
}

fn search_path(name: &str) -> Option<std::path::PathBuf> {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════
// Chrome 进程管理
// ═══════════════════════════════════════════════════════════

struct ChromeProcess {
    child: Child,
    ws_url: String,
    _temp_dir: tempfile::TempDir,
}

impl ChromeProcess {
    /// 启动 Chrome，返回 WebSocket URL
    async fn launch(chrome_path: Option<&Path>, timeout: Duration) -> Result<Self> {
        let chrome = chrome_path.map(|p| p.to_path_buf())
            .or_else(find_chrome_in_path)
            .ok_or_else(|| anyhow::anyhow!("找不到 Chrome/Chromium 可执行文件"))?;

        // 使用独立临时用户数据目录
        let temp_dir = tempfile::tempdir()
            .map_err(|e| anyhow::anyhow!("无法创建临时目录: {}", e))?;
        let data_dir = temp_dir.path().join("chrome-profile");

        let mut cmd = TokioCommand::new(&chrome);
        cmd.args([
            "--headless",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-software-rasterizer",
            "--disable-dev-shm-usage",
            "--disable-extensions",
            "--disable-background-networking",
            "--no-first-run",
            "--hide-scrollbars",
            "--mute-audio",
            &format!("--user-data-dir={}", data_dir.display()),
            "--remote-debugging-port=0", // 随机端口
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("无法启动 Chrome: {}", e))?;

        let stderr = child.stderr.take()
            .ok_or_else(|| anyhow::anyhow!("无法获取 Chrome stderr"))?;

        // 从 stderr 中读取 WebSocket URL
        let ws_url = read_ws_url(stderr, timeout).await?;

        Ok(Self { child, ws_url, _temp_dir: temp_dir })
    }

    async fn kill(&mut self) -> Result<()> {
        self.child.kill().await?;
        self.child.wait().await?;
        Ok(())
    }
}

/// 从 Chrome stderr 中读取 "DevTools listening on ws://..."
async fn read_ws_url(mut stderr: impl tokio::io::AsyncRead + Unpin + Send, timeout: Duration) -> Result<String> {
    use tokio::io::AsyncBufReadExt;
    let reader = tokio::io::BufReader::new(&mut stderr);
    let mut lines = reader.lines();

    let _start = std::time::Instant::now();
    while let Some(line) = tokio::time::timeout(timeout, lines.next_line()).await
        .map_err(|_| anyhow::anyhow!("Chrome 启动超时 ({}s 内未输出 WebSocket URL)", timeout.as_secs()))?
        .map_err(|e| anyhow::anyhow!("读取 Chrome stderr 失败: {}", e))?
    {
        if let Some(ws) = line.rsplit_once("listening on ") {
            let url = ws.1.trim();
            if url.starts_with("ws") && url.contains("devtools/browser") {
                return Ok(url.to_string());
            }
        }
    }

    bail!("Chrome stderr 已关闭，未找到 WebSocket URL");
}

// ═══════════════════════════════════════════════════════════
// CDP WebSocket 会话
// ═══════════════════════════════════════════════════════════

struct CdpSession {
    write: tokio::sync::Mutex<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    next_id: AtomicU32,
    session_id: Option<String>,
}

impl CdpSession {
    /// 连接到 Chrome DevTools
    async fn connect(ws_url: &str, timeout: Duration) -> Result<Self> {
        let connect_fut = connect_async(ws_url);
        let (ws, _) = tokio::time::timeout(timeout, connect_fut)
            .await
            .map_err(|_| anyhow::anyhow!("WebSocket 连接超时 ({}s)", timeout.as_secs()))?
            .map_err(|e| anyhow::anyhow!("WebSocket 连接失败: {}", e))?;

        Ok(Self {
            write: tokio::sync::Mutex::new(ws),
            next_id: AtomicU32::new(1),
            session_id: None,
        })
    }

    /// 发送 CDP 命令并等待响应
    async fn call(&self, method: &str, params: Option<Value>, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut request = json!({
            "id": id,
            "id": id,
            "method": method,
            "params": params.unwrap_or(json!({})),
        });
        // 如果有 page session，添加到消息中
        if let Some(sid) = &self.session_id {
            request["sessionId"] = json!(sid);
        }

        // 发送命令
        {
            let mut ws = self.write.lock().await;
            futures::SinkExt::send(&mut *ws, Message::Text(request.to_string())).await
                .map_err(|e| anyhow::anyhow!("WebSocket 发送失败: {}", e))?;
        }

        // 等待匹配的响应
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                bail!("CDP 命令 '{}' 超时 ({}s)", method, timeout.as_secs());
            }

            let msg = {
                let mut ws = self.write.lock().await;
                tokio::time::timeout(
                    Duration::from_secs(1),
                    futures::StreamExt::next(&mut *ws),
                ).await
            };

            match msg {
                Ok(Some(Ok(Message::Text(text)))) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                        if val.get("id").and_then(|v| v.as_i64()) == Some(id as i64) {
                            if let Some(error) = val.get("error") {
                                let msg = error.get("message").and_then(|v| v.as_str()).unwrap_or("unknown error");
                                bail!("CDP 命令 '{}' 失败: {}", method, msg);
                            }
                            return Ok(val.get("result").cloned().unwrap_or(json!({})));
                        }
                    }
                }
                Ok(Some(Ok(Message::Ping(_)))) => {
                    let mut ws = self.write.lock().await;
                    let _ = futures::SinkExt::send(&mut *ws, Message::Pong(vec![])).await;
                }
                Ok(Some(Ok(Message::Close(_)))) => bail!("CDP WebSocket 连接已关闭"),
                Ok(Some(Err(e))) => bail!("WebSocket 接收错误: {}", e),
                Ok(None) => bail!("CDP WebSocket 连接已关闭"),
                Err(_) => {} // timeout polling, continue
                _ => {}
            }
        }
    }

    /// 等待特定 CDP 事件
    async fn wait_for_event(&self, method: &str, timeout: Duration) -> Result<Value> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                bail!("等待事件 '{}' 超时 ({}s)", method, timeout.as_secs());
            }

            let msg = {
                let mut ws = self.write.lock().await;
                tokio::time::timeout(
                    Duration::from_secs(1),
                    futures::StreamExt::next(&mut *ws),
                ).await
            };

            match msg {
                Ok(Some(Ok(Message::Text(text)))) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                        if val.get("method").and_then(|v| v.as_str()) == Some(method) {
                            return Ok(val.get("params").cloned().unwrap_or(json!({})));
                        }
                    }
                }
                Ok(Some(Ok(Message::Ping(_)))) => {
                    let mut ws = self.write.lock().await;
                    let _ = futures::SinkExt::send(&mut *ws, Message::Pong(vec![])).await;
                }
                Ok(Some(Ok(Message::Close(_)))) => bail!("CDP WebSocket 连接已关闭"),
                Ok(Some(Err(e))) => bail!("WebSocket 接收错误: {}", e),
                Ok(None) => bail!("CDP WebSocket 连接已关闭"),
                Err(_) => {}
                _ => {}
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════
// PDF 渲染核心逻辑
// ═══════════════════════════════════════════════════════════

async fn render_inner(
    cdp: &mut CdpSession,
    html_path: &Path,
    output_pdf: &Path,
    cfg: &PdfOptions,
    timeout: Duration,
) -> Result<()> {
    let url = file_url(html_path)?;
    log::info!("轻量 CDP: 使用 Target.createTarget 创建页面并导航...");

    // 1. 创建新页面并导航到目标 URL
    let create_result = cdp.call("Target.createTarget", Some(json!({
        "url": url.clone(),
    })), timeout).await;
    let create_result = match create_result {
        Ok(r) => r,
        Err(e) => {
            log::warn!("轻量 CDP: createTarget 失败: {:?}", e);
            return Err(e.context(format!("无法创建页面 (url={})", url)));
        }
    };

    let target_id = create_result.get("targetId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("createTarget 缺少 targetId"))?;

    log::info!("轻量 CDP: 页面已创建 (targetId={}), 正在附加会话...", target_id);

    // 2. 附加到页面并获取 sessionId
    let attach_result = cdp.call("Target.attachToTarget", Some(json!({
        "targetId": target_id,
        "flatten": true,
    })), timeout).await
        .context("无法附加到页面")?;

    let session_id = attach_result.get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("attachToTarget 缺少 sessionId"))?;

    cdp.session_id = Some(session_id.to_string());
    log::info!("轻量 CDP: 会话已附加 (sessionId={})", session_id);

    // 2.5 启用 Page 域（才能收到页面加载事件）
    log::info!("轻量 CDP: 启用 Page 域...");
    if let Err(e) = cdp.call("Page.enable", None, timeout).await {
        log::warn!("轻量 CDP: Page.enable 失败 ({}，继续)", e);
    }

    // 3. 等待页面加载完成
    log::info!("轻量 CDP: 等待页面加载...");
    let load_result = tokio::time::timeout(
        Duration::from_secs(cfg.timeout),
        cdp.wait_for_event("Page.frameStoppedLoading", timeout),
    ).await;

    match load_result {
        Ok(Ok(_)) => log::info!("轻量 CDP: 页面加载完成"),
        Ok(Err(e)) => log::warn!("轻量 CDP: 页面加载事件异常: {} (继续)", e),
        Err(_) => log::warn!("轻量 CDP: 页面加载超时 ({}s, 继续)", cfg.timeout),
    }

    // 短暂等待确保渲染完成
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 4. 调用 Page.printToPDF
    log::info!("轻量 CDP: 调用 Page.printToPDF...");
    let pdf_params = build_print_to_pdf_json(cfg);
    let result = cdp.call("Page.printToPDF", Some(pdf_params), Duration::from_secs(cfg.timeout)).await
        .context("Page.printToPDF 调用失败")?;

    // 5. 解码 base64 PDF
    let pdf_base64 = result.get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("printToPDF 响应中缺少 data 字段"))?;

    let pdf_data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        pdf_base64,
    ).map_err(|e| anyhow::anyhow!("base64 解码失败: {}", e))?;

    log::info!("轻量 CDP: PDF 数据已接收, {} 字节", pdf_data.len());

    // 6. 写入输出文件
    std::fs::write(output_pdf, &pdf_data)?;
    log::info!("轻量 CDP: PDF 已保存到: {}", output_pdf.display());

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// PDF 参数构建
// ═══════════════════════════════════════════════════════════

/// 构建 Page.printToPDF 的 JSON 参数
fn build_print_to_pdf_json(cfg: &PdfOptions) -> Value {
    let mut params = json!({});

    let hf_enabled = cfg.header_footer_enabled();
    let use_cdp_hf = hf_enabled && cfg.use_native_header_footer;

    // 页面几何
    params["paperWidth"] = json!(cfg.paper_width);
    params["paperHeight"] = json!(cfg.paper_height);
    params["marginTop"] = json!(cfg.margin_top);
    params["marginBottom"] = json!(cfg.margin_bottom);
    params["marginLeft"] = json!(cfg.margin_left);
    params["marginRight"] = json!(cfg.margin_right);

    if cfg.landscape {
        params["landscape"] = json!(true);
    }
    if (cfg.scale - 1.0).abs() > f64::EPSILON {
        params["scale"] = json!(cfg.scale);
    }
    if cfg.prefer_css_page_size {
        params["preferCSSPageSize"] = json!(true);
    }
    if cfg.print_background {
        params["printBackground"] = json!(true);
    }
    if !cfg.page_range.is_empty() {
        params["pageRanges"] = json!(cfg.page_range);
    }

    // 页眉/页脚
    if use_cdp_hf {
        params["displayHeaderFooter"] = json!(true);
        if !cfg.header_template.is_empty() {
            params["headerTemplate"] = json!(cfg.header_template);
        }
        if !cfg.footer_template.is_empty() {
            params["footerTemplate"] = json!(cfg.footer_template);
        }
    }

    // PDF 标签
    if cfg.generate_tagged_pdf {
        params["generateTaggedPDF"] = json!(true);
    }

    // 文档大纲由后处理模块负责
    params["generateDocumentOutline"] = json!(false);

    params
}
