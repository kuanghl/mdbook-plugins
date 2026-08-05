//! mdbook-pdf-preview-assets — 在 html 渲染后把 pdf.js viewer 资源复制到构建输出。
//!
//! 为什么需要独立 renderer：`mdbook-pdf-preview` 是 preprocessor，其执行先于
//! html renderer，而 html renderer 会**清空输出目录**（`fs::remove_dir_content`），
//! preprocessor 阶段复制过去的文件会被删掉；且 mdbook 只复制 additional-js 引用的
//! 文件，viewer 的整套资源（web/ + build/）不会被复制。
//!
//! 本 renderer 在 html renderer **之后**运行（book.toml 中把
//! `[output.pdf-preview-assets]` 配置在 `[output.html]` 之后），把书目录
//! `assets/pdfviewer/` 下的 `web/`（viewer.html 等）与 `build/pdf.mjs`、
//! `build/pdf.worker.mjs` 复制到 html 输出目录，供 pdf-preview.js 的 viewer
//! 模式使用（不污染 src/）。
//!
//! book.toml 配置：
//! ```toml
//! [preprocessor.pdf-preview]
//! command = "mdbook-plugins pdf-preview"
//! mode = "pdfjs"
//!
//! [output.html]
//! additional-js = ["./assets/pdfviewer/pdf-preview.js"]
//!
//! [output.pdf-preview-assets]
//! command = "mdbook-plugins pdf-preview-assets"
//! ```

use mdbook_renderer::{RenderContext, Renderer};

pub struct PdfPreviewAssetsRenderer;

impl Renderer for PdfPreviewAssetsRenderer {
    fn name(&self) -> &str {
        "mdbook-pdf-preview-assets"
    }

    fn render(&self, ctx: &RenderContext) -> anyhow::Result<()> {
        // 每个 renderer 的 destination 是 {build_dir}/<renderer名>，
        // 而 pdf-preview.js 期望从 HTML 页面相对定位 assets/pdfviewer/，
        // 所以写到 html renderer 的输出目录（destination 的父目录 + "html"）。
        let html_dir = ctx
            .destination
            .parent()
            .map(|p| p.join("html"))
            .unwrap_or_else(|| ctx.destination.clone());
        let dst_root = html_dir.join("assets").join("pdfviewer");
        let dst_build = dst_root.join("build");

        // 完整 pdf.js viewer（web/viewer.html 等）+ 核心库（build/pdf.mjs + pdf.worker.mjs）：
        // 从书目录 assets/pdfviewer/ 复制。mdbook 只复制 additional-js 引用的文件，
        // viewer 的整套资源需要这里在 html 渲染后复制到输出，避免污染 src/。
        let src_root = ctx.root.join("assets").join("pdfviewer");
        let src_web = src_root.join("web");
        if src_web.is_dir() {
            copy_dir_all(&src_web, &dst_root.join("web"))?;
        }
        let src_build = src_root.join("build");
        if src_build.is_dir() {
            std::fs::create_dir_all(&dst_build)?;
            for name in ["pdf.mjs", "pdf.worker.mjs"] {
                let f = src_build.join(name);
                if f.is_file() {
                    std::fs::copy(&f, dst_build.join(name))?;
                }
            }
        }
        Ok(())
    }
}

/// 递归复制目录（跳过文件锁/临时文件）
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else if ty.is_file() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 运行 mdbook-pdf-preview-assets 渲染器（main.rs 调用的入口）
pub fn run() -> anyhow::Result<()> {
    let renderer = PdfPreviewAssetsRenderer;
    crate::utils::run_renderer(&renderer)
}
