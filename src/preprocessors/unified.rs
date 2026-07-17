//! mdbook-plugins — 统一预处理器
//!
//! 将所有子预处理器合并为一个，通过 book.toml 中的命名空间配置来控制
//! 启用哪些子插件及其各自的参数。
//!
//! # book.toml 配置示例
//!
//! ```toml
//! [preprocessor.mdbook-plugins]
//! # 启用所有子插件（默认）
//! # enabled = ["all"]
//!
//! # 或只启用特定子插件
//! # enabled = ["alerts", "toc", "katex", "admonish", "mermaid", "echarts",
//! #             "pikchr", "emojicodes", "image-viewer", "embedify",
//! #             "kroki", "langtabs", "svgbob", "wavedrom"]
//!
//! # 子插件各自的配置通过命名空间传递
//! # [preprocessor.mdbook-plugins.katex]
//! # no-css = true
//! # include-src = true
//! ```

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};

use crate::preprocessors;

/// 所有支持的子插件名称列表（按推荐执行顺序排列）
const ALL_PLUGINS: &[&str] = &[
    "toc",          // <!-- toc --> 目录
    "emojicodes",   // :emoji: shortcode 替换
    "katex",        // $..$ 和 $$..$$ LaTeX 公式
    "admonish",     // ```admonish 提示框
    "alerts",       // > [!NOTE] GitHub 风格提醒
    "mermaid",      // ```mermaid 图表
    "pikchr",       // ```pikchr 图
    "svgbob",       // ```bob ASCII art → SVG
    "wavedrom",     // ```wavedrom 时序图
    "kroki",        // ```kroki-* 图（需网络请求）
    "langtabs",     // <!-- langtabs --> 语言标签页
    "embedify",     // {% youtube/codepen/giscus %} 嵌入
    "echarts",      // ```echarts / ```bytefield / ```latex / ```typst
    "image-viewer", // 图片点击放大查看器（应最后执行）
];

pub struct UnifiedPreprocessor;

impl Preprocessor for UnifiedPreprocessor {
    fn name(&self) -> &str {
        "mdbook-plugins"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer != "not-supported")
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        // 解析配置，确定启用的子插件列表及命名空间配置
        let config = parse_plugins_config(ctx);
        let enabled = config.enabled;

        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                chapter.content = process_chapter(&enabled, &config.namespaced, &chapter.content);
            }
        });

        Ok(book)
    }
}

/// 解析后的插件配置
struct PluginsConfig {
    /// 按 ALL_PLUGINS 顺序排列的启用于插件名称列表
    enabled: Vec<&'static str>,
    /// 每个子插件的命名空间配置（key=插件名, value=TOML table）
    namespaced: NamespacedConfig,
}

/// 命名空间配置：为每个启用的子插件保留其专属配置段
///
/// 对应 book.toml 中的 `[preprocessor.mdbook-plugins.<name>]`
type NamespacedConfig = toml::value::Table;

/// 从 book.toml 解析 `[preprocessor.mdbook-plugins]` 配置段
///
/// 返回 PluginsConfig，其中：
/// - `enabled`：启用的子插件列表（由 `enabled` 键控制，默认全部）
/// - `namespaced`：包含所有键值对，子插件命名空间（如 katex、toc）作为子表保留
fn parse_plugins_config(ctx: &PreprocessorContext) -> PluginsConfig {
    let config: Option<toml::value::Table> = ctx.config
        .get("preprocessor.mdbook-plugins")
        .ok()
        .flatten();
    let config = match config {
        Some(cfg) => cfg,
        None => {
            return PluginsConfig {
                enabled: ALL_PLUGINS.to_vec(),
                namespaced: toml::value::Table::new(),
            };
        }
    };

    // 解析 enabled 列表
    let enabled = parse_enabled_list(&config);

    // 构建命名空间配置：从 config 中提取所有子表（排除 enabled 等顶层元字段）
    let mut namespaced = toml::value::Table::new();
    let meta_keys = ["enabled", "after", "before", "renderer", "command"];
    for (key, value) in config.iter() {
        // 只保留属于子插件的命名空间配置（且该子插件在 ALL_PLUGINS 中）
        if !meta_keys.contains(&key.as_str()) && ALL_PLUGINS.contains(&key.as_str()) {
            namespaced.insert(key.clone(), value.clone());
        }
    }

    PluginsConfig { enabled, namespaced }
}

/// 解析 book.toml 中的 enabled 配置
///
/// 返回按 ALL_PLUGINS 顺序过滤后的启用于插件名称列表。
/// 如果未配置 enabled 或包含 "all"，则返回全部子插件。
fn parse_enabled_list(config: &toml::value::Table) -> Vec<&'static str> {
    let enabled = match config.get("enabled") {
        Some(v) => v,
        None => return ALL_PLUGINS.to_vec(),
    };

    let enabled_list: Vec<String> = match enabled.as_array() {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
            .collect(),
        None => return ALL_PLUGINS.to_vec(),
    };

    // 如果包含 "all"，启用全部
    if enabled_list.iter().any(|s| s == "all") {
        return ALL_PLUGINS.to_vec();
    }

    // 按 ALL_PLUGINS 顺序过滤
    ALL_PLUGINS
        .iter()
        .filter(|name| enabled_list.iter().any(|e| e == *name))
        .copied()
        .collect()
}

/// 获取指定子插件的命名空间配置
///
/// 返回 `None` 表示该子插件没有命名空间配置段。
fn get_plugin_config<'a>(
    namespaced: &'a NamespacedConfig,
    plugin_name: &str,
) -> Option<&'a toml::Value> {
    namespaced.get(plugin_name)
}

/// 按顺序对章节内容运行所有启用的子插件，传递命名空间配置
fn process_chapter(
    enabled: &[&'static str],
    namespaced: &NamespacedConfig,
    content: &str,
) -> String {
    let mut result = content.to_string();

    for plugin in enabled {
        let plugin_cfg = get_plugin_config(namespaced, plugin);
        result = match *plugin {
            "toc" => preprocessors::toc::process_content(&result, plugin_cfg),
            "emojicodes" => preprocessors::emojicodes::process_content(&result, plugin_cfg),
            "katex" => preprocessors::katex::process_content(&result, plugin_cfg),
            "admonish" => preprocessors::admonish::process_content(&result, plugin_cfg),
            "alerts" => preprocessors::alerts::process_content(&result, plugin_cfg),
            "mermaid" => preprocessors::mermaid::process_content(&result, plugin_cfg),
            "pikchr" => preprocessors::pikchr::process_content(&result, plugin_cfg),
            "svgbob" => preprocessors::svgbob::process_content(&result, plugin_cfg),
            "wavedrom" => preprocessors::wavedrom::process_content(&result, plugin_cfg),
            "kroki" => preprocessors::kroki::process_content(&result, plugin_cfg),
            "langtabs" => preprocessors::langtabs::process_content(&result, plugin_cfg),
            "embedify" => preprocessors::embedify::process_content(&result, plugin_cfg),
            "echarts" => preprocessors::echarts::process_content(&result, plugin_cfg),
            "image-viewer" => preprocessors::image_viewer::process_content(&result, plugin_cfg),
            other => {
                log::warn!("mdbook-plugins: 未知的子插件 '{}'，已跳过", other);
                result
            }
        };
    }

    result
}

/// 运行统一预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = UnifiedPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
