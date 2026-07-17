//! mdbook-plugins — 单二进制多插件分发器
//!
//! 通过 argv[0]（符号链接名称）自动路由到对应 mdbook 插件的处理逻辑。
//! 遵循 mdbook 标准协议：
//!   - 无参数或 "supports <renderer>" → 预处理器协议
//!   - 其他参数 → 传递给具体插件处理

use std::path::Path;
use std::process;

fn main() {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("warn"));

    // 确定调用名称 —— 从 argv[0] 提取文件名
    let bin_name = std::env::args().next()
        .map(|p| Path::new(&p).file_stem().unwrap_or_default().to_string_lossy().into_owned())
        .unwrap_or_default();

    // 确定插件名称
    // 如果 argv[1] 形如 "mdbook-xxx" 则当作插件名覆盖
    // 否则从 argv[0]（symlink 名）推断
    let plugin_name = std::env::args().nth(1)
        .filter(|a| a.starts_with("mdbook-"))
        .unwrap_or_else(|| {
            bin_name.clone()
        });

    if !plugin_name.starts_with("mdbook-") {
        eprintln!("mdbook-plugins: 无法确定插件名称");
        eprintln!("  用法: 以 mdbook-<name> 命名符号链接，或传递 mdbook-<name> 作为首个参数");
        eprintln!("  支持的插件: {}", mdbook_plugins::PLUGIN_NAMES.join(", "));
        process::exit(1);
    }

    // 收集剩余参数（排除 argv[0] 和可能的插件名参数）
    let args: Vec<String> = std::env::args().skip(1)
        .filter(|a| !a.starts_with("mdbook-") || a == &plugin_name)
        .collect();

    // 路由到对应插件
    run_plugin(&plugin_name, &args);
}

fn run_plugin(name: &str, args: &[String]) {
    // 判断是预处理器还是渲染器
    let _is_preprocessor = matches!(name,
        "mdbook-admonish" | "mdbook-alerts" | "mdbook-echarts" |
        "mdbook-emojicodes" | "mdbook-embedify" | "mdbook-image-viewer" | "mdbook-katex" |
        "mdbook-kroki-preprocessor" | "mdbook-langtabs" | "mdbook-mermaid" |
        "mdbook-pikchr" | "mdbook-svgbob" | "mdbook-toc" | "mdbook-wavedrom-rs"
    );

    let _is_renderer = matches!(name,
        "mdbook-asciidoc" | "mdbook-linkcheck" | "mdbook-office" | "mdbook-pdf"
    );

    // 处理 `supports <renderer>` 子命令
    if _is_preprocessor && args.first().map(|s| s.as_str()) == Some("supports") {
        let renderer = args.get(1).map(|s| s.as_str()).unwrap_or("");
        let pre: Box<dyn mdbook::preprocess::Preprocessor> = match name {
            "mdbook-admonish" => Box::new(mdbook_plugins::preprocessors::admonish::AdmonishPreprocessor),
            "mdbook-alerts" => Box::new(mdbook_plugins::preprocessors::alerts::AlertsPreprocessor),
            "mdbook-echarts" => Box::new(mdbook_plugins::preprocessors::echarts::ChartPreprocessor),
            "mdbook-emojicodes" => Box::new(mdbook_plugins::preprocessors::emojicodes::EmojiCodesPreprocessor),
            "mdbook-embedify" => Box::new(mdbook_plugins::preprocessors::embedify::EmbedifyPreprocessor),
            "mdbook-image-viewer" => Box::new(mdbook_plugins::preprocessors::image_viewer::ImageViewerPreprocessor),
            "mdbook-katex" => Box::new(mdbook_plugins::preprocessors::katex::KatexPreprocessor),
            "mdbook-kroki-preprocessor" => Box::new(mdbook_plugins::preprocessors::kroki::KrokiPreprocessor),
            "mdbook-langtabs" => Box::new(mdbook_plugins::preprocessors::langtabs::LangTabsPreprocessor),
            "mdbook-mermaid" => Box::new(mdbook_plugins::preprocessors::mermaid::MermaidPreprocessor),
            "mdbook-pikchr" => Box::new(mdbook_plugins::preprocessors::pikchr::PikchrPreprocessor),
            "mdbook-svgbob" => Box::new(mdbook_plugins::preprocessors::svgbob::SvgbobPreprocessor),
            "mdbook-toc" => Box::new(mdbook_plugins::preprocessors::toc::TocPreprocessor),
            "mdbook-wavedrom-rs" => Box::new(mdbook_plugins::preprocessors::wavedrom::WavedromPreprocessor),
            _ => process::exit(1),
        };
        if pre.supports_renderer(renderer) {
            process::exit(0);
        } else {
            process::exit(1);
        }
        // 上面的 process::exit 已终止，不会继续
    }

    // 正常执行插件
    let result = match name {
        "mdbook-admonish" => mdbook_plugins::preprocessors::admonish::run(),
        "mdbook-alerts" => mdbook_plugins::preprocessors::alerts::run(),
        "mdbook-echarts" => mdbook_plugins::preprocessors::echarts::run(),
        "mdbook-emojicodes" => mdbook_plugins::preprocessors::emojicodes::run(),
        "mdbook-embedify" => mdbook_plugins::preprocessors::embedify::run(),
        "mdbook-image-viewer" => mdbook_plugins::preprocessors::image_viewer::run(),
        "mdbook-katex" => mdbook_plugins::preprocessors::katex::run(),
        "mdbook-kroki-preprocessor" => mdbook_plugins::preprocessors::kroki::run(),
        "mdbook-langtabs" => mdbook_plugins::preprocessors::langtabs::run(),
        "mdbook-mermaid" => mdbook_plugins::preprocessors::mermaid::run(),
        "mdbook-pikchr" => mdbook_plugins::preprocessors::pikchr::run(),
        "mdbook-svgbob" => mdbook_plugins::preprocessors::svgbob::run(),
        "mdbook-toc" => mdbook_plugins::preprocessors::toc::run(),
        "mdbook-wavedrom-rs" => mdbook_plugins::preprocessors::wavedrom::run(),
        "mdbook-asciidoc" => mdbook_plugins::renderers::asciidoc::run(),
        "mdbook-linkcheck" => mdbook_plugins::renderers::linkcheck::run(),
        "mdbook-office" => mdbook_plugins::renderers::office::run(),
        "mdbook-pdf" => mdbook_plugins::renderers::pdf::run(),
        _ => {
            eprintln!("mdbook-plugins: 未知的插件 '{}'", name);
            process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("mdbook-plugins ({}): 错误: {}", name, e);
        process::exit(1);
    }
}
