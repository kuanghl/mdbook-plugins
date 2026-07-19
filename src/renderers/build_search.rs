//! mdbook-build-search — 中文搜索索引构建渲染器
//!
//! 在 book.toml 中配置:
//!   [output.build-search]
//!   command = "mdbook-plugins build-search"
//!
//! mdbook 会在 HTML 渲染完成后自动调用此渲染器，
//! 对生成的 HTML 文件建立中文 bigram 搜索索引。

use mdbook_core::errors::Error;
use mdbook_renderer::{RenderContext, Renderer};

pub struct BuildSearchRenderer;

impl Renderer for BuildSearchRenderer {
    fn name(&self) -> &str {
        "build-search"
    }

    fn render(&self, ctx: &RenderContext) -> Result<(), Error> {
        // build-search 的输出目录是 books/zz-build-search，
        // 但需要处理的是 HTML 目录（books/html）。
        // 从 ctx.destination 的父目录推断 HTML 目录。
        let html_dir = ctx.destination
            .parent()
            .map(|p| p.join("html"))
            .filter(|p| p.exists())
            .unwrap_or_else(|| ctx.root.join("book").join("html"));

        log::info!("build-search: 处理 HTML 目录: {}", html_dir.display());
        crate::build_search::run(&html_dir.to_string_lossy())
            .map_err(|e| Error::msg(format!("build-search 失败: {}", e)))?;

        // 删除空的 zz-build-search 输出目录（实际工作已写入 html/）
        let _ = std::fs::remove_dir_all(&ctx.destination);

        Ok(())
    }
}

pub fn run() -> anyhow::Result<()> {
    let renderer = BuildSearchRenderer;
    crate::utils::run_renderer(&renderer)
}
