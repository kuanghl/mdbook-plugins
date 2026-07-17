//! mdbook-plugins — 单二进制多插件支持的 mdbook 插件集合
//!
//! 本库将所有 mdbook 预处理器和渲染器合并为一个项目，
//! 通过 argv[0] 或子命令分发到对应插件的处理逻辑。

pub mod preprocessors;
pub mod renderers;
pub mod utils;

/// 插件注册表 —— 所有支持的插件名称
pub const PLUGIN_NAMES: &[&str] = &[
    // 预处理器
    "mdbook-admonish",
    "mdbook-alerts",
    "mdbook-echarts",
    "mdbook-emojicodes",
    "mdbook-embedify",
    "mdbook-image-viewer",
    "mdbook-katex",
    "mdbook-kroki-preprocessor",
    "mdbook-langtabs",
    "mdbook-mermaid",
    "mdbook-pikchr",
    "mdbook-svgbob",
    "mdbook-toc",
    "mdbook-wavedrom-rs",
    // 渲染器
    "mdbook-asciidoc",
    "mdbook-linkcheck",
    "mdbook-office",
    "mdbook-pdf",
];
