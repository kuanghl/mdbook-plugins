//! mdbook-plugins — 单二进制多插件分发器
//!
//! 支持两种调用方式：
//!   1. 符号链接：mdbook-<name> [args...]
//!   2. 命令参数：mdbook-plugins <name> [args...]
//!
//! 其中 <name> 为子插件短名称（如 katex、toc、pdf 等）。

use std::path::Path;
use std::process;

/// 所有已知插件的短名称列表（不含 mdbook- 前缀）
const KNOWN_SHORT_NAMES: &[&str] = &[
    // 预处理器
    "admonish", "alerts", "echarts", "emojicodes", "embedify",
    "image-viewer", "katex", "kroki-preprocessor", "langtabs",
    "mermaid", "pikchr", "plugins", "svgbob", "toc", "wavedrom-rs",
    // 渲染器
    "asciidoc", "linkcheck", "office", "pdf",
    // 独立工具
    "build-search",
];

fn main() {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("warn"));

    let args: Vec<String> = std::env::args().collect();

    // CLI 直接调用：mdbook-plugins build-search <html-dir>
    // （区别于 Renderer 模式：后者通过 stdin 接收 RenderContext，无目录参数）
    if args.len() >= 3 && args[1] == "build-search" {
        let html_dir = &args[2];
        if let Err(e) = mdbook_plugins::build_search::run(html_dir) {
            eprintln!("mdbook-plugins build-search: 错误: {}", e);
            process::exit(1);
        }
        return;
    }

    // 解析插件名称和剩余参数
    let (plugin_name, plugin_args) = resolve_plugin(&args);

    if !plugin_name.starts_with("mdbook-") {
        eprintln!("mdbook-plugins: 无法确定插件名称");
        eprintln!("  用法: mdbook-plugins <name> [args...]");
        eprintln!("  或: 创建符号链接 mdbook-<name> -> mdbook-plugins");
        eprintln!("  支持的插件: {}", mdbook_plugins::PLUGIN_NAMES.join(", "));
        process::exit(1);
    }

    run_plugin(&plugin_name, &plugin_args);
}

/// 解析命令行参数，返回 (插件全名, 剩余参数)
///
/// 解析规则：
/// - argv[1] 是已知短名称（如 katex）→ 插件名 = "mdbook-<name>"
/// - argv[1] 是 mdbook-xxx 格式 → 直接作为插件名
/// - 否则从 argv[0]（符号链接名）推断
fn resolve_plugin(args: &[String]) -> (String, Vec<String>) {
    let bin_name = args.first()
        .map(|p| Path::new(p).file_stem().unwrap_or_default().to_string_lossy().into_owned())
        .unwrap_or_default();

    if args.len() < 2 {
        // 无参数：从 argv[0] 推断
        return (bin_name, vec![]);
    }

    let first_arg = &args[1];

    // 情况1: argv[1] 是已知短名称 → 组装为 mdbook-<name>
    if KNOWN_SHORT_NAMES.contains(&first_arg.as_str()) {
        let plugin_name = format!("mdbook-{}", first_arg);
        let plugin_args: Vec<String> = args[2..].to_vec();
        return (plugin_name, plugin_args);
    }

    // 情况2: argv[1] 是 mdbook-xxx 格式 → 直接作为插件名
    if first_arg.starts_with("mdbook-") {
        let plugin_args: Vec<String> = args[2..].to_vec();
        return (first_arg.clone(), plugin_args);
    }

    // 情况3: 从 argv[0]（符号链接）推断
    let plugin_args: Vec<String> = args[1..].to_vec();
    (bin_name, plugin_args)
}

fn run_plugin(name: &str, args: &[String]) {
    // 判断是预处理器还是渲染器
    let _is_preprocessor = matches!(name,
        "mdbook-admonish" | "mdbook-alerts" | "mdbook-echarts" |
        "mdbook-emojicodes" | "mdbook-embedify" | "mdbook-image-viewer" | "mdbook-katex" |
        "mdbook-kroki-preprocessor" | "mdbook-langtabs" | "mdbook-mermaid" |
        "mdbook-pikchr" | "mdbook-plugins" | "mdbook-svgbob" | "mdbook-toc" | "mdbook-wavedrom-rs"
    );

    let _is_renderer = matches!(name,
        "mdbook-asciidoc" | "mdbook-linkcheck" | "mdbook-office" | "mdbook-pdf" | "mdbook-build-search"
    );

    // 处理 `supports <renderer>` 子命令
    if _is_preprocessor && args.first().map(|s| s.as_str()) == Some("supports") {
        let renderer = args.get(1).map(|s| s.as_str()).unwrap_or("");
        let pre: Box<dyn mdbook_preprocessor::Preprocessor> = match name {
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
            "mdbook-plugins" => Box::new(mdbook_plugins::preprocessors::unified::UnifiedPreprocessor),
            "mdbook-svgbob" => Box::new(mdbook_plugins::preprocessors::svgbob::SvgbobPreprocessor),
            "mdbook-toc" => Box::new(mdbook_plugins::preprocessors::toc::TocPreprocessor),
            "mdbook-wavedrom-rs" => Box::new(mdbook_plugins::preprocessors::wavedrom::WavedromPreprocessor),
            _ => process::exit(1),
        };
        match pre.supports_renderer(renderer) {
            Ok(true) => process::exit(0),
            _ => process::exit(1),
        }
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
        "mdbook-plugins" => mdbook_plugins::preprocessors::unified::run(),
        "mdbook-svgbob" => mdbook_plugins::preprocessors::svgbob::run(),
        "mdbook-toc" => mdbook_plugins::preprocessors::toc::run(),
        "mdbook-wavedrom-rs" => mdbook_plugins::preprocessors::wavedrom::run(),
        "mdbook-asciidoc" => mdbook_plugins::renderers::asciidoc::run(),
        "mdbook-linkcheck" => mdbook_plugins::renderers::linkcheck::run(),
        "mdbook-build-search" => mdbook_plugins::renderers::build_search::run(),
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
