//! 轻量级 CDP 客户端 — 直接通过 WebSocket 通信，替代 chromiumoxide
//!
//! 设计原则：
//! - 只实现 PDF 生成需要的 CDP 命令（Page.enable, Page.navigate, Page.printToPDF）
//! - 完全控制超时，不受第三方库限制
//! - 单文件实现，约 300 行

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
                    "轻量 CDP 第 {}/{} 次尝试失败: {}. 清空进程池后 500ms 重试...",
                    attempt, max_attempts, e
                );
                invalidate_pool_chrome();
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

    let timeout = Duration::from_secs(cfg.timeout);

    // 1. 从进程池获取 Chrome WebSocket URL（自动启动/复用）
    let ws_url = acquire_chrome_ws_url(cfg, timeout).await?;

    // 2. 连接 CDP（每次新建会话，WS 连接成本 ~10-50ms）
    log::info!("轻量 CDP: 连接 WebSocket...");
    let mut cdp = CdpSession::connect(&ws_url, timeout).await?;

    // 3. 渲染 PDF
    let result = render_inner(&mut cdp, temp_html_path, output_pdf, cfg, timeout).await;

    // 4. 不关闭 Chrome — 放回进程池供下次复用
    //    （闲置超时由 acquire_chrome_ws_url 在下次获取时处理）
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
// Chrome 进程池（复用 Chrome 进程，避免反复启动/销毁）
// ═══════════════════════════════════════════════════════════

/// Chrome 闲置超时秒数 — 超过此时间未使用则自动关闭
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// 进程池中的 Chrome 实例状态
struct PooledChrome {
    child: Child,
    ws_url: String,
    _temp_dir: tempfile::TempDir,
    last_used: Instant,
}

/// 全局 Chrome 进程池
static CHROME_POOL: once_cell::sync::Lazy<Mutex<Option<PooledChrome>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// 从进程池获取 Chrome WebSocket URL
///
/// 策略：
/// 1. 若池中有实例且未超时 → 直接复用，减少 0.5-3s 启动时间
/// 2. 若池中有实例但已超时 → 关闭旧进程，启动新实例
/// 3. 若池为空 → 启动新实例并存入池中
async fn acquire_chrome_ws_url(cfg: &PdfOptions, timeout: Duration) -> Result<String> {
    // ── 尝试复用池中实例 ──
    let pooled_to_kill = {
        let mut pool = CHROME_POOL.lock().unwrap();
        if let Some(ref mut inner) = *pool {
            if inner.last_used.elapsed() > POOL_IDLE_TIMEOUT {
                // 超时：从池中取出，稍后统一清理
                Some(pool.take().unwrap())
            } else {
                log::info!(
                    "复用 Chrome 进程池中的实例 (闲置 {:.1}s)",
                    inner.last_used.elapsed().as_secs_f64()
                );
                inner.last_used = Instant::now();
                return Ok(inner.ws_url.clone());
            }
        } else {
            None
        }
    }; // ⚠️ MutexGuard 在此处释放，不跨 .await 持有

    // 清理超时的旧实例（在锁外执行 .await）
    if let Some(mut p) = pooled_to_kill {
        log::info!("Chrome 进程闲置超时，关闭旧进程...");
        let _ = p.child.kill().await;
        let _ = p.child.wait().await;
        // p 在此处 drop，temp_dir 自动清理
    }

    // ── 启动新 Chrome 进程 ──
    log::info!("进程池为空，启动新的 Chrome 实例...");

    let chrome = resolve_chrome_path(cfg)
        .or_else(find_chrome_in_path)
        .ok_or_else(|| anyhow::anyhow!("找不到 Chrome/Chromium 可执行文件"))?;

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

    let ws_url = read_ws_url(stderr, timeout).await?;

    // ── 存入进程池（锁外完成启动后） ──
    {
        let mut pool = CHROME_POOL.lock().unwrap();
        *pool = Some(PooledChrome {
            child,
            ws_url: ws_url.clone(),
            _temp_dir: temp_dir,
            last_used: Instant::now(),
        });
    }

    log::info!("新 Chrome 实例已启动并存入进程池");
    Ok(ws_url)
}

/// 使池中 Chrome 实例失效（渲染失败时调用，确保下次重试启动新实例）
fn invalidate_pool_chrome() {
    let mut pool = CHROME_POOL.lock().unwrap();
    if let Some(p) = pool.take() {
        if let Some(pid) = p.child.id() {
            // 先终止进程，再丢弃资源（child handle + temp dir）
            unsafe { libc::kill(pid as i32, libc::SIGKILL); }
        }
        drop(p);
        log::info!("已终止失效的 Chrome 进程");
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

/// 等待内容加载哨兵元素出现（替代固定 300ms 等待）
///
/// 通过 CDP `Runtime.evaluate` 轮询 DOM 中由 `inject_js` 注入的
/// `#content-has-all-loaded-for-mdbook-pdf-generation` 元素。
/// 该哨兵在页面 load 事件 + 可能存在的 MathJax 完成 + 100ms 后出现。
///
/// - 典型情况（无 MathJax）：~100-150ms 内返回
/// - 有 MathJax：等待 MathJax 排版完成后返回
/// - 超时保护：最多等待 `timeout`，超时后仍继续 PDF 生成
async fn wait_for_content_sentinel(cdp: &CdpSession, timeout: Duration) {
    let check_expr =
        "document.getElementById('content-has-all-loaded-for-mdbook-pdf-generation') !== null";
    let start = Instant::now();

    loop {
        if start.elapsed() > timeout {
            log::warn!(
                "轻量 CDP: 内容加载哨兵等待超时 ({}s)，继续 PDF 生成",
                timeout.as_secs()
            );
            return;
        }

        match cdp
            .call(
                "Runtime.evaluate",
                Some(json!({
                    "expression": check_expr,
                    "returnByValue": true,
                    "awaitPromise": false,
                })),
                Duration::from_secs(5),
            )
            .await
        {
            Ok(val) => {
                let found = val
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if found {
                    log::info!(
                        "轻量 CDP: 内容加载哨兵已出现（耗时 {:.0}ms）",
                        start.elapsed().as_millis()
                    );
                    return;
                }
            }
            Err(e) => {
                log::warn!("轻量 CDP: 检查内容加载哨兵失败: {} (继续)", e);
            }
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
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

    // 3.5 等待内容加载哨兵（替代固定 300ms 等待）
    log::info!("轻量 CDP: 等待内容加载哨兵...");
    wait_for_content_sentinel(cdp, Duration::from_secs(cfg.timeout)).await;

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
