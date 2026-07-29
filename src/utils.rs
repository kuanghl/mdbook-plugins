//! 插件通用的工具函数

use std::io::IsTerminal;

/// 标准的 mdbook 预处理器入口：从 stdin 读取，处理，写入 stdout
pub fn run_preprocessor<P: mdbook_preprocessor::Preprocessor>(
    pre: &P,
) -> anyhow::Result<()> {
    let (ctx, book) = mdbook_preprocessor::parse_input(std::io::stdin())?;

    let book_version = semver::Version::parse(&ctx.mdbook_version)?;
    let version_req = semver::VersionReq::parse(mdbook_preprocessor::MDBOOK_VERSION)?;
    if !version_req.matches(&book_version) {
        log::debug!(
            "{} was built against mdbook v{}, but running with v{}",
            pre.name(),
            mdbook_preprocessor::MDBOOK_VERSION,
            ctx.mdbook_version,
        );
    }

    let processed = pre.run(&ctx, book)?;
    serde_json::to_writer(std::io::stdout(), &processed)?;
    Ok(())
}

/// 标准的 supports_renderer 处理
pub fn handle_supports(pre: &dyn mdbook_preprocessor::Preprocessor, renderer: &str) {
    match pre.supports_renderer(renderer) {
        Ok(true) => std::process::exit(0),
        _ => std::process::exit(1),
    }
}

/// 标准的 mdbook 渲染器入口：从 stdin 读取 RenderContext，处理
pub fn run_renderer<R: mdbook_renderer::Renderer>(renderer: &R) -> anyhow::Result<()> {
    let ctx = mdbook_renderer::RenderContext::from_json(std::io::stdin())?;
    renderer.render(&ctx)?;
    Ok(())
}

/// 计算从章节路径到 images 目录的相对路径前缀
///
/// 例如: "index.md" → "./images/", "test/7.md" → "../images/"
pub fn relative_svg_prefix(chapter_path: &std::path::Path) -> String {
    let depth = chapter_path.parent().map(|p| p.components().count()).unwrap_or(0);
    if depth == 0 {
        "./images/".to_string()
    } else {
        let parents: Vec<&str> = std::iter::repeat("..").take(depth).collect();
        format!("{}/images/", parents.join("/"))
    }
}

/// 独立输出状态信息到 stderr，格式: " \x1b[32m INFO\x1b[0m <message>"
///
/// 与 print_progress 风格一致，终端中 INFO 显示为绿色。
/// 不受 RUST_LOG 级别影响，始终输出。
/// 输出到文件/管道时自动降级为纯文本 " INFO"。
pub fn print_status(msg: &str) {
    let info_prefix = if std::io::stderr().is_terminal() {
        "\x1b[32m INFO\x1b[0m"
    } else {
        " INFO"
    };
    eprintln!("{} {}", info_prefix, msg);
}
