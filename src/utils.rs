//! 插件通用的工具函数

/// 标准的 mdbook 预处理器入口：从 stdin 读取，处理，写入 stdout
pub fn run_preprocessor<P: mdbook::preprocess::Preprocessor>(
    pre: &P,
) -> anyhow::Result<()> {
    let (ctx, book) = mdbook::preprocess::CmdPreprocessor::parse_input(std::io::stdin())?;

    let book_version = semver::Version::parse(&ctx.mdbook_version)?;
    let version_req = semver::VersionReq::parse(mdbook::MDBOOK_VERSION)?;
    if !version_req.matches(&book_version) {
        log::debug!(
            "{} was built against mdbook v{}, but running with v{}",
            pre.name(),
            mdbook::MDBOOK_VERSION,
            ctx.mdbook_version,
        );
    }

    let processed = pre.run(&ctx, book)?;
    serde_json::to_writer(std::io::stdout(), &processed)?;
    Ok(())
}

/// 标准的 supports_renderer 处理
pub fn handle_supports(pre: &dyn mdbook::preprocess::Preprocessor, renderer: &str) {
    if pre.supports_renderer(renderer) {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

/// 标准的 mdbook 渲染器入口：从 stdin 读取 RenderContext，处理
pub fn run_renderer<R: mdbook::renderer::Renderer>(renderer: &R) -> anyhow::Result<()> {
    let ctx = mdbook::renderer::RenderContext::from_json(std::io::stdin())?;
    renderer.render(&ctx)?;
    Ok(())
}
