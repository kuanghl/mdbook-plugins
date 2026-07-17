//! mdbook-pikchr — Pikchr 图渲染预处理器
//!
//! 将 ```pikchr 代码块渲染为内联 SVG。
//!
//! ## 关于 pikchr C 库
//!
//! pikchr 是一个将 PIC 语言翻译为 SVG 的 C 库。
//! C 函数签名 (vendor/pikchr.c:7929):
//!
//! ```c
//! char *pikchr(
//!   const char *zText,     /* 零结尾的 PIKCHR 源文本 */
//!   const char *zClass,    /* <svg> 的 class 属性值 */
//!   unsigned int mFlags,   /* 渲染行为 flags */
//!   int *pnWidth,          /* 输出宽度 (可为 NULL) */
//!   int *pnHeight          /* 输出高度 (可为 NULL) */
//! );
//! ```
//!
//! 返回值是 malloc 分配的 SVG 字符串，调用者必须 free()。

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};

pub struct PikchrPreprocessor;

extern "C" {
    /// pikchr C 库入口函数
    ///
    /// 将 PIKCHR 语言文本渲染为 SVG。
    /// 返回 malloc 分配的字符串，调用者需用 libc::free 释放。
    fn pikchr(
        zText: *const std::ffi::c_char,
        zClass: *const std::ffi::c_char,
        mFlags: std::ffi::c_uint,
        pnWidth: *mut std::ffi::c_int,
        pnHeight: *mut std::ffi::c_int,
    ) -> *mut std::ffi::c_char;
}

impl Preprocessor for PikchrPreprocessor {
    fn name(&self) -> &str {
        "mdbook-pikchr"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer != "not-supported")
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                chapter.content = process_chapter(&chapter.content);
            }
        });
        Ok(book)
    }
}

fn process_chapter(content: &str) -> String {
    // 使用字符串操作而非 pulldown-cmark 事件流，避免 _ => {} 丢弃事件
    // 以及避免注入的 HTML 被转义
    let mut output = String::with_capacity(content.len() + 4096);
    let mut pos = 0;
    let bytes = content.as_bytes();

    while pos < bytes.len() {
        // 查找 ``` 标记
        if pos + 3 < bytes.len() && &bytes[pos..pos+3] == b"```" {
            let info_start = pos + 3;
            let mut info_end = info_start;

            // 读取 fence info string（到行尾）
            while info_end < bytes.len() && bytes[info_end] != b'\n' {
                info_end += 1;
            }
            let info = content[info_start..info_end].trim();
            let newline_after_info = if info_end < bytes.len() { 1 } else { 0 };

            if info.starts_with("pikchr") {
                let align = info.trim_start_matches("pikchr").trim().to_string();
                // 找到代码块内容（从 info_end+1 开始）
                let content_start = info_end + newline_after_info;
                // 找下一个 ``` 闭合 fence
                let fence_end = find_closing_fence(bytes, content_start);
                let pikchr_content = if fence_end > content_start {
                    // fence_end 指向闭合 ``` 之后的位置。
                    // 代码块内容在 content_start 到闭合 ``` 之前。
                    // 从 fence_end 往回跳过闭合 fence 和前面的换行。
                    let start = content_start;
                    // 越过闭合 ``` (3 个反引号)
                    let mut content_end = if fence_end >= 3 { fence_end - 3 } else { fence_end };
                    // 越过 ``` 前面的换行
                    if content_end > start && bytes[content_end-1] == b'\n' {
                        content_end -= 1;
                        if content_end > start && bytes[content_end-1] == b'\r' {
                            content_end -= 1;
                        }
                    }
                    if start < content_end {
                        let c = content[start..content_end].to_string();
                        c
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                match render_pikchr(&pikchr_content, &align) {
                    Ok(svg) => {
                        output.push_str(&svg);
                        // 添加空行分隔，确保后续 Markdown 被正确渲染
                        output.push_str("\n\n");
                    }
                    Err(e) => {
                        log::warn!("pikchr 渲染失败: {}", e);
                        output.push_str("<pre><code class=\"language-pikchr\">");
                        output.push_str(&pikchr_content);
                        output.push_str("</code></pre>\n");
                    }
                }

                if fence_end > 0 {
                    // 跳过闭合 fence 及其后的换行
                    pos = fence_end;
                    // 跳过可能的 \n 或 \r\n
                    while pos < bytes.len() && (bytes[pos] == b'\n' || bytes[pos] == b'\r') {
                        pos += 1;
                    }
                } else {
                    // 没有闭合 fence，直接复制剩余内容
                    output.push_str(&content[pos..]);
                    break;
                }
            } else {
                // 非 pikchr 代码块，原样复制
                let line_end = find_line_end(bytes, info_end);
                output.push_str(&content[pos..=line_end]);
                pos = line_end + 1;
            }
        } else {
            // 普通内容，复制到下一个 ``` 或文件末尾
            let next_fence = content[pos..].find("```");
            match next_fence {
                Some(n) => {
                    output.push_str(&content[pos..pos+n]);
                    pos = pos + n;
                }
                None => {
                    output.push_str(&content[pos..]);
                    break;
                }
            }
        }
    }

    output
}

/// 从 start 位置开始查找闭合的 ``` fence
fn find_closing_fence(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i + 3 <= bytes.len() {
        if &bytes[i..i+3] == b"```" {
            return i + 3; // 返回 ``` 之后的位置
        }
        i += 1;
    }
    0 // 没找到
}

/// 找到行尾（\n 或 \r\n）
fn find_line_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            return i;
        }
        if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i+1] == b'\n' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len() - 1
}

pub fn render_pikchr(script: &str, _align: &str) -> Result<String, Box<dyn std::error::Error>> {
    let c_script = std::ffi::CString::new(script)?;
    let z_class = std::ffi::CString::new("pikchr")?;
    let mut width: std::ffi::c_int = 0;
    let mut height: std::ffi::c_int = 0;

    let result_ptr = unsafe {
        pikchr(
            c_script.as_ptr(),
            z_class.as_ptr(),
            0, // mFlags: 0 = 默认, PIKCHR_PLAINTEXT_ERRORS=1, PIKCHR_DARK_MODE=2
            &mut width,
            &mut height,
        )
    };

    if result_ptr.is_null() {
        return Err("pikchr returned NULL (可能内存分配失败)".into());
    }

    // 将 C 字符串转为 Rust String
    let svg_output = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result_ptr);
        let s = c_str.to_string_lossy().into_owned();
        // 释放 pikchr 用 malloc() 分配的内存
        libc::free(result_ptr as *mut std::ffi::c_void);
        s
    };

    log::debug!("pikchr 返回 (前300字): {:?}", &svg_output[..std::cmp::min(300, svg_output.len())]);

    // 如果 SVG 内容包含错误信息（pikchr 在出错时返回含 <pre> 的 HTML）
    // 检查是否包含 <svg 标签来判断是否成功
    if !svg_output.contains("<svg") && svg_output.contains("<pre>") {
        log::warn!("pikchr 渲染返回错误信息: {}", svg_output);
        return Err("pikchr 语法错误".into());
    }

    // 包裹 SVG
    Ok(format!(
        r#"<div class="pikchr-wrapper">{}</div>"#,
        svg_output
    ))
}

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
    process_chapter(content)
}

/// 运行 mdbook-pikchr 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = PikchrPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
