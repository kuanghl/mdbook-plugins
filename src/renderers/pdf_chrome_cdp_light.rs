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
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use super::pdf::PdfOptions;
use crate::utils::print_progress;

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
                    "轻量 CDP 第 {}/{} 次尝试失败: {:#}. 清空进程池后 500ms 重试...",
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
    print_progress(1, 14, "Writing temp HTML");
    std::fs::write(temp_html_path, html_content)?;

    let timeout = Duration::from_secs(cfg.timeout);

    // 1. 从进程池获取 Chrome WebSocket URL（自动启动/复用）
    print_progress(2, 14, "Starting Chrome");
    let ws_url = acquire_chrome_ws_url(cfg, timeout).await?;

    // 2. 连接 CDP（每次新建会话，WS 连接成本 ~10-50ms）
    print_progress(3, 14, "Connecting DevTools");
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

/// 通过 Browser.getVersion 快速验证 Chrome 进程是否健康
async fn verify_chrome_health(ws_url: &str, timeout: Duration) -> bool {
    match tokio::time::timeout(timeout, async {
        let mut ws_config = WebSocketConfig::default();
        ws_config.max_frame_size = Some(256 * 1024 * 1024);
        ws_config.max_message_size = Some(256 * 1024 * 1024);
        let (ws, _) = connect_async_with_config(ws_url, Some(ws_config), false)
            .await
            .map_err(|_| "连接失败")?;
        let mut write = ws;

        // 发送 Browser.getVersion（无需 session，最快验证方式）
        let req = serde_json::json!({"id": 1, "method": "Browser.getVersion"});
        futures::SinkExt::send(&mut write, Message::Text(req.to_string().into()))
            .await
            .map_err(|_| "发送失败")?;

        // 读取响应
        if let Some(Ok(Message::Text(text))) = futures::StreamExt::next(&mut write).await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                if val.get("id").and_then(|v| v.as_i64()) == Some(1) && val.get("error").is_none()
                {
                    return Ok(());
                }
            }
        }
        Err("响应无效")
    })
    .await
    {
        Ok(Ok(())) => true,
        _ => false,
    }
}

/// 从进程池获取 Chrome WebSocket URL
///
/// 策略：
/// 1. 若池中有实例且未超时 → 先验证 WebSocket 健康，健康则复用
/// 2. 若健康检查失败 → 清理池中实例，启动新进程
/// 3. 若池中有实例但已超时 → 关闭旧进程，启动新实例
/// 4. 若池为空 → 启动新实例并存入池中
async fn acquire_chrome_ws_url(cfg: &PdfOptions, timeout: Duration) -> Result<String> {
    // ── 尝试复用池中实例（含健康检查） ──
    // 先取出池中实例的 ws_url（如果有且未超时），稍后验证健康
    let pooled_ws_url = {
        let mut pool = CHROME_POOL.lock().unwrap();
        match pool.as_mut() {
            Some(inner) if inner.last_used.elapsed() <= POOL_IDLE_TIMEOUT => {
                Some(inner.ws_url.clone())
            }
            _ => None,
        }
    }; // ⚠️ MutexGuard 在此处释放，不跨 .await 持有

    if let Some(ref ws_url) = pooled_ws_url {
        if verify_chrome_health(ws_url, Duration::from_secs(5)).await {
            // 健康：更新 last_used 后复用
            let mut pool = CHROME_POOL.lock().unwrap();
            if let Some(ref mut inner) = *pool {
                if inner.ws_url == *ws_url {
                    let idle = inner.last_used.elapsed().as_secs_f64();
                    inner.last_used = Instant::now();
                    log::debug!(
                        "复用 Chrome 进程池中的实例 (闲置 {:.1}s)",
                        idle
                    );
                    return Ok(ws_url.clone());
                }
            }
            // 池已被其他操作修改，继续向下启动新实例
        } else {
            log::warn!("Chrome 进程池中的实例不健康，将重新启动");
            invalidate_pool_chrome();
        }
    }

    // ── 清理超时实例 ──
    let pooled_to_kill = {
        let mut pool = CHROME_POOL.lock().unwrap();
        if let Some(ref mut inner) = *pool {
            if inner.last_used.elapsed() > POOL_IDLE_TIMEOUT {
                Some(pool.take().unwrap())
            } else {
                None
            }
        } else {
            None
        }
    }; // ⚠️ MutexGuard 在此处释放

    if let Some(mut p) = pooled_to_kill {
        log::debug!("Chrome 进程闲置超时，关闭旧进程...");
        let _ = p.child.kill().await;
        let _ = p.child.wait().await;
        // p 在此处 drop，temp_dir 自动清理
    }

    // ── 启动新 Chrome 进程 ──
    log::debug!("进程池为空，启动新的 Chrome 实例...");

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

    log::debug!("新 Chrome 实例已启动并存入进程池");
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
        log::debug!("已终止失效的 Chrome 进程");
    }
}

/// 关闭 Chrome 进程池 — 正常退出时调用，确保子进程被回收
///
/// 调用方应在整个渲染流程结束后（成功或失败）调用此函数。
/// 与 `invalidate_pool_chrome` 不同，本函数不会记录"失效"日志，
/// 属于正常关闭操作。
pub fn shutdown_pool() {
    let mut pool = CHROME_POOL.lock().unwrap();
    if let Some(p) = pool.take() {
        // 先尝试优雅终止（SIGTERM），等待一小段时间后再强杀
        if let Some(pid) = p.child.id() {
            unsafe { libc::kill(pid as i32, libc::SIGTERM); }
            // 给 Chrome 一点时间优雅退出
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        // drop child 时会尝试 wait（非阻塞）
        drop(p);
        log::debug!("Chrome 进程池已关闭");
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
        let mut ws_config = WebSocketConfig::default();
        // 默认 max_frame_size=16MB 对大型 PDF(base64) 不够，设为 256MB
        ws_config.max_frame_size = Some(256 * 1024 * 1024);
        ws_config.max_message_size = Some(256 * 1024 * 1024);
        let connect_fut = connect_async_with_config(ws_url, Some(ws_config), false);
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
            futures::SinkExt::send(&mut *ws, Message::Text(request.to_string().into())).await
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
                    let _ = futures::SinkExt::send(&mut *ws, Message::Pong(vec![].into())).await;
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
                    let _ = futures::SinkExt::send(&mut *ws, Message::Pong(vec![].into())).await;
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

/// 等待内容加载哨兵元素出现
///
/// 通过 CDP `Runtime.evaluate` 执行返回 Promise 的 JS 脚本，
/// 内部使用 MutationObserver 监听 DOM 变化，哨兵出现时立即 resolve。
/// 不在 JS 内部设硬超时（避免大文档加载慢时提前放弃），
/// 由 Rust 端循环 + 总超时 `timeout` 控制重试。
///
/// - 快路径：哨兵出现 → Promise resolve → 立即返回
/// - 慢路径：cdp.call 超时（120s）→ 重试 → 直到总超时
/// 等待内容哨兵，同时报告中间进度
async fn wait_for_content_sentinel(cdp: &CdpSession, timeout: Duration) {
    let start = Instant::now();
    let mut last_reported_step = 0u8;
    
    // 进度检查脚本 - 检查各个阶段的完成状态
    let progress_script = r#"
        (function() {
            var status = {
                dom_ready: !!window.__mdbookPdfDomReady,
                emoji_done: !!window.__mdbookPdfEmojiDone,
                fonts_ready: !!window.__mdbookPdfFontsReady,
                content_ready: !!window.__mdbookPdfContentReady
            };
            return status;
        })()
    "#;

    // 主等待循环 - 定期检查进度并更新进度条
    loop {
        if start.elapsed() > timeout {
            log::warn!(
                "轻量 CDP: 内容加载等待超时 ({}s)，继续 PDF 生成",
                timeout.as_secs(),
            );
            return;
        }

        // 检查进度
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            cdp.call(
                "Runtime.evaluate",
                Some(json!({
                    "expression": progress_script.trim(),
                    "returnByValue": true,
                })),
                Duration::from_secs(5),
            )
        ).await;

        if let Ok(Ok(val)) = result {
            if let Some(result) = val.get("result").and_then(|r| r.get("value")) {
                let dom_ready = result.get("dom_ready").and_then(|v| v.as_bool()).unwrap_or(false);
                let emoji_done = result.get("emoji_done").and_then(|v| v.as_bool()).unwrap_or(false);
                let fonts_ready = result.get("fonts_ready").and_then(|v| v.as_bool()).unwrap_or(false);
                let content_ready = result.get("content_ready").and_then(|v| v.as_bool()).unwrap_or(false);

                // 根据当前状态计算进度步骤（5-12步）
                let (current_step, current_label) = if content_ready {
                    (12u8, "Content ready")
                } else if fonts_ready {
                    (11u8, "Finalizing content")
                } else if emoji_done {
                    (10u8, "Loading fonts")
                } else if dom_ready {
                    (9u8, "Processing emoji")
                } else {
                    (5u8, "Waiting for DOM")
                };

                // 只在步骤变化时更新进度条
                if current_step > last_reported_step {
                    print_progress(current_step, 14, current_label);
                    last_reported_step = current_step;
                }

                // 内容完全就绪
                if content_ready {
                    log::debug!(
                        "轻量 CDP: 内容加载完成（耗时 {:.0}ms）",
                        start.elapsed().as_millis(),
                    );
                    return;
                }
            }
        }

        // 等待一小段时间再检查
        tokio::time::sleep(Duration::from_millis(300)).await;
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
    print_progress(4, 14, "Loading page");
    log::debug!("轻量 CDP: 创建页面并导航到: {}", url);
    let t0 = std::time::Instant::now();

    // ── 阶段 1：创建页面并导航到目标 URL ──
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
    log::debug!("[timing] createTarget: {:.0}ms", t0.elapsed().as_millis());

    let target_id = create_result.get("targetId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("createTarget 缺少 targetId"))?;

    log::debug!("轻量 CDP: 页面已创建 (targetId={}), 正在附加会话...", target_id);

    // ── 阶段 2：附加到页面并完成配置 ──
    let attach_result = cdp.call("Target.attachToTarget", Some(json!({
        "targetId": target_id,
        "flatten": true,
    })), timeout).await
        .context("无法附加到页面")?;

    let session_id = attach_result.get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("attachToTarget 缺少 sessionId"))?;

    cdp.session_id = Some(session_id.to_string());
    log::debug!("轻量 CDP: 会话已附加 (sessionId={})", session_id);
    log::debug!("[timing] attachToTarget: {:.0}ms", t0.elapsed().as_millis());

    log::debug!("轻量 CDP: 启用 Page 域...");
    if let Err(e) = cdp.call("Page.enable", None, timeout).await {
        log::warn!("轻量 CDP: Page.enable 失败 ({}，继续)", e);
    }
    log::debug!("[timing] page+network setup: {:.0}ms", t0.elapsed().as_millis());

    // ── 阶段 3：等待页面加载完成（短超时，等不到就靠哨兵） ──
    // Page.frameStoppedLoading 事件可能在之前的 call()（Page.enable 等）
    // 中被消费并丢弃。1s 短超时：收到就提前进入哨兵，收不到直接走哨兵。
    // 哨兵阶段的 window.load 自然覆盖了页面加载等待，不依赖此事件。
    log::debug!("轻量 CDP: 尝试等待页面加载事件 (1s 超时)...");
    let load_result = tokio::time::timeout(
        Duration::from_secs(1),
        cdp.wait_for_event("Page.frameStoppedLoading", Duration::from_secs(1)),
    ).await;

    match load_result {
        Ok(Ok(_)) => log::debug!("轻量 CDP: 页面加载完成"),
        Ok(Err(_e)) => log::debug!("轻量 CDP: 页面加载事件超时 (1s)，继续等待哨兵"),
        Err(_) => log::debug!("轻量 CDP: 页面加载事件超时 (1s)，继续等待哨兵"),
    }
    log::debug!("[timing] frameStoppedLoading wait: {:.0}ms", t0.elapsed().as_millis());

    // ── 阶段 4：等待内容哨兵（进度在 wait_for_content_sentinel 中动态更新 5→12） ──
    wait_for_content_sentinel(cdp, timeout).await;
    log::debug!("[timing] sentinel wait: {:.0}ms", t0.elapsed().as_millis());

    // ── 阶段 5：调用 Page.printToPDF ──
    print_progress(12, 14, "Generating PDF");
    log::debug!("轻量 CDP: 调用 Page.printToPDF...");
    let pdf_params = build_print_to_pdf_json(cfg);

    // 尝试 printToPDF，如果带 generateTaggedPDF 参数失败则降级重试
    let result = match cdp
        .call("Page.printToPDF", Some(pdf_params.clone()), Duration::from_secs(cfg.timeout))
        .await
    {
        Ok(r) => r,
        Err(e) if cfg.generate_tagged_pdf => {
            log::warn!(
                "轻量 CDP: printToPDF 带 generateTaggedPDF 参数失败 ({:#}), 尝试移除参数后重试...",
                e
            );
            // 移除 generateTaggedPDF 参数后重试
            if let Some(obj) = pdf_params.as_object() {
                let mut fallback = obj.clone();
                fallback.remove("generateTaggedPDF");
                cdp.call(
                    "Page.printToPDF",
                    Some(serde_json::Value::Object(fallback)),
                    Duration::from_secs(cfg.timeout),
                )
                .await
                .context("Page.printToPDF 调用失败")?
            } else {
                return Err(e).context("Page.printToPDF 调用失败");
            }
        }
        Err(e) => return Err(e).context("Page.printToPDF 调用失败"),
    };
    log::debug!("[timing] printToPDF call: {:.0}ms", t0.elapsed().as_millis());

    // 5. 解码 base64 PDF
    let pdf_base64 = result.get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("printToPDF 响应中缺少 data 字段"))?;

    let pdf_data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        pdf_base64,
    ).map_err(|e| anyhow::anyhow!("base64 解码失败: {}", e))?;

    log::debug!("轻量 CDP: PDF 数据已接收, {} 字节", pdf_data.len());

    // 6. 写入输出文件
    std::fs::write(output_pdf, &pdf_data)?;
    log::debug!("轻量 CDP: PDF 已保存到: {}", output_pdf.display());
    log::debug!("[timing] TOTAL render_inner: {:.0}ms", t0.elapsed().as_millis());

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
