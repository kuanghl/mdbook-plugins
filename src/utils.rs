//! 插件通用的工具函数

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
