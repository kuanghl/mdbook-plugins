//! mdbook-echarts — 统一图表预处理器
//!
//! 参考 prj_mdbook/mdbook-kroki/mdbook-echarts/src/echarts/mod.rs
//!
//! 一次性处理所有代码块类型：
//! - ```echarts    → 唯一化变量名 + DOMContentLoaded
//! - ```bob       → svgbob 内联 SVG
//! - ```bytefield → bytefield 容器
//! - ```latex tex  → <latex-js> 包裹
//! - ```latex tikz → <details> 折叠 + <img> SVG
//! - ```pikchr     → pikchr 内联 SVG
//! - ```typst      → typst crate 编译 → SVG + PDF 文件
//! - ```wavedrom   → WaveDrom script

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use svgbob::Render;
use uuid::Uuid;

pub struct ChartPreprocessor;

impl Preprocessor for ChartPreprocessor {
    fn name(&self) -> &str {
        "mdbook-echarts"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer == "html")
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        // Build svg output dir: {root}/src/images/ (both PDF and SVG stored here)
        // mdbook copies non-markdown files from src/ to the build output automatically.
        let svg_dir = ctx
            .root
            .join("src")
            .join("images");

        // Tectonic format cache: {root}/{build_dir}/Tectonic/
        let tectonic_cache_dir = ctx
            .root
            .join(&ctx.config.build.build_dir)
            .join("Tectonic");

        // 章节级进度条（全英文）：统计章节总数，逐章推进并显示每章耗时，
        // 用于分析 HTML 构建阶段（preprocess）的性能瓶颈。
        let total_chapters = book
            .iter()
            .filter(|item| matches!(item, BookItem::Chapter(_)))
            .count();
        let mut chapters_done = 0usize;

        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                let chapter_start = std::time::Instant::now();
                let chapter_name = chapter.name.clone();
                let chapter_path = chapter.path.clone().unwrap_or_else(|| PathBuf::from("index.md"));
                chapter.content =
                    process_chapter(&chapter.name, &chapter.content, &svg_dir, &chapter_path, &tectonic_cache_dir);
                chapters_done += 1;
                crate::utils::print_progress(
                    chapters_done as u8,
                    total_chapters.max(1) as u8,
                    &format!(
                        "Chapter {}/{}: {} ({})",
                        chapters_done,
                        total_chapters,
                        chapter_name,
                        crate::utils::format_elapsed(chapter_start.elapsed())
                    ),
                );
            }
        });

        // 清理 src/images 中不再被引用的旧 diagram 缓存文件：
        // md 内容更新 → 内容 hash 变 → 新 {hash}.svg/.pdf 生成，旧文件无人引用。
        // 这里收集所有章节当前引用的 hash（data-pdf-hash + 文件名引用），
        // 删除「hash 命名模式 且 ∉ 引用集合」的生成物——缓存命中逻辑不受影响
        // （未变化的代码块 hash 相同，直接复用文件），旧文件不再累积。
        let referenced = collect_referenced_diagram_hashes(&book);
        cleanup_orphan_diagrams(&svg_dir, &referenced);

        Ok(book)
    }
}

fn process_chapter(_name: &str, content: &str, svg_dir: &Path, chapter_path: &Path, tectonic_cache_dir: &Path) -> String {
    let mut s = content.to_string();

    // 按顺序处理各种代码块（先处理 pikchr/svgbob 再处理其他）

    // 1) ```echarts
    let re = Regex::new(r"```echarts((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = echarts_gen_html(mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 2) ```bob
    let re = Regex::new(r"```bob((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = svgbob_gen_html(mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 3) ```bytefield
    let re = Regex::new(r"```bytefield((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = bytefield_gen_html(mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 4) ```latex tex (LaTeX 文档 → tectonic SVG；失败回退 latex-js)
    let re = Regex::new(r"```latex tex((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let t = std::time::Instant::now();
        let buf = latex_gen_file(mat.as_str(), &svg_dir, chapter_path, tectonic_cache_dir);
        crate::utils::print_status(&format!(
            "  LaTeX document compiled ({})",
            crate::utils::format_elapsed(t.elapsed())
        ));
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 5) ```latex tikz (TikZ 图片 → 并行编译 tectonic PDF → hayro-svg SVG, 最大并发 4)
    let re = Regex::new(r"```latex tikz((.*\n)+?)?```").unwrap();
    {
        let s_clone = s.clone();
        let all_matches: Vec<&str> = re.find_iter(s_clone.as_str()).map(|m| m.as_str()).collect();

        // 分批并行编译，限制最大并发数避免 OOM
        const TIKZ_MAX_CONCURRENT: usize = 4;
        let mut results = Vec::with_capacity(all_matches.len());
        for chunk in all_matches.chunks(TIKZ_MAX_CONCURRENT) {
            let chunk_results: Vec<String> = chunk
                .par_iter()
                .map(|mat_str| {
                    let t = std::time::Instant::now();
                    let r = tikz_gen_file(mat_str, svg_dir, chapter_path, tectonic_cache_dir);
                    crate::utils::print_status(&format!(
                        "  TikZ diagram compiled ({})",
                        crate::utils::format_elapsed(t.elapsed())
                    ));
                    r
                })
                .collect();
            results.extend(chunk_results);
        }

        for (mat_str, result) in all_matches.into_iter().zip(results.into_iter()) {
            s = s.replace(mat_str, &result);
        }
    }

    // 6) ```pikchr
    let re = Regex::new(r"```pikchr((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = pikchr_gen_html(mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 7) ```typst (Typst 图片 → 并行编译 typst crate → SVG + PDF, 最大并发 4)
    let re = Regex::new(r"```typst((.*\n)+?)?```").unwrap();
    {
        let s_clone = s.clone();
        let all_matches: Vec<&str> = re.find_iter(s_clone.as_str()).map(|m| m.as_str()).collect();

        const TYPST_MAX_CONCURRENT: usize = 4;
        let mut results = Vec::with_capacity(all_matches.len());
        for chunk in all_matches.chunks(TYPST_MAX_CONCURRENT) {
            let chunk_results: Vec<String> = chunk
                .par_iter()
                .map(|mat_str| {
                    let t = std::time::Instant::now();
                    let r = typst_gen_file(mat_str, svg_dir, chapter_path, tectonic_cache_dir);
                    crate::utils::print_status(&format!(
                        "  Typst diagram compiled ({})",
                        crate::utils::format_elapsed(t.elapsed())
                    ));
                    r
                })
                .collect();
            results.extend(chunk_results);
        }

        for (mat_str, result) in all_matches.into_iter().zip(results.into_iter()) {
            s = s.replace(mat_str, &result);
        }
    }

    // 8) ```wavedrom
    let re = Regex::new(r"```wavedrom((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = wavedrom_gen_html(mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    s
}

/// 清理代码块标记，提取内容
fn clean_codeblock(mat_str: &str, start_tag: &str) -> String {
    mat_str
        .replace(start_tag, "")
        .replace("```", "")
        .trim()
        .to_string()
}

/// ===== echarts =====
fn echarts_gen_html(mat_str: &str) -> String {
    let mut content = clean_codeblock(mat_str, "```echarts");
    let uuid = Uuid::new_v4().to_string().replace('-', "");

    // 去除空行，防止 pulldown-cmark 将生成的 HTML 视为 type 6 块
    // 并在空白行处提前截断，导致 script 内 JS 被当作 Markdown 解析（包裹 <p> 标签）。
    content = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    content = content.replace("chartDom", &format!("chartDom_{}", uuid));
    content = content.replace("myChart", &format!("chart_{}", uuid));
    content = content.replace("document.getElementById('main')",
        &format!("document.getElementById('{}')", uuid));
    content = content.replace("document.getElementById(\"main\")",
        &format!("document.getElementById(\"{}\")", uuid));
    content = content.replace("--k", "(k-=1,k)");

    format!(
        r#"<div>
    <div id="{}" style="height: 500px; text-align: center;">
<script type="text/javascript">
        document.addEventListener('DOMContentLoaded', function() {{
            {}
        }});
</script>
    </div>
</div>"#,
        uuid, content
    )
}

/// ===== svgbob =====
fn svgbob_gen_html(mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```bob");
    if content.is_empty() {
        return String::new();
    }

    let settings = svgbob::Settings::default();
    let cb = svgbob::CellBuffer::from(content.as_str());
    let (svg_node, _, _): (svgbob::Node<()>, f32, f32) = cb.get_node_with_size(&settings);

    let mut source = String::new();
    if let Err(e) = svg_node.render_with_indent(&mut source, 0, true) {
        log::warn!("svgbob 渲染失败: {}", e);
        return format!(r#"<pre><code class="language-bob">{}</code></pre>"#, content);
    }

    let uuid = Uuid::new_v4().to_string().replace('-', "");
    let svg = source.replace("svgbob", &format!("svgbob_{}", uuid));

    format!(
        r#"<pre class="diagram-svgbob" style="text-align: center;">
{}
</pre>"#,
        svg
    )
}

/// ===== bytefield =====
fn bytefield_gen_html(mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```bytefield");
    // 消除所有空行，避免 pulldown-cmark 提前结束 HTML 块（匹配 mdbook-demo 行为）
    let re = Regex::new(r"\n{2,}").unwrap();
    let content = re.replace_all(&content, "\n");
    // HTML 转义：bytefield 语法可能含 `<`，直接内联会触发 pulldown-cmark
    // 的 "Saw - in state TagOpen" 警告
    let escaped = crate::utils::escape_xml(&content);
    format!(
        r#"<div>
    <div id="CommonMark-bytefield" style="text-align: center;">
    <pre tabindex="0"><code class="language-bytefield" data-lang="bytefield">
{}
    </code></pre>
    </div>
</div>"#,
        escaped
    )
}

/// ===== latex tex (LaTeX 文档) =====
///
/// 优先走与 TikZ 相同的管线：tectonic (XeTeX) → PDF → hayro-svg → SVG，
/// 以 `<img>` 引用 `{hash}.svg`（`data-pdf-hash` 关联中间 PDF 供页面合并）。
/// 该管线失败（完整文档与 standalone 文档类不兼容、宏包缺失等）时，
/// **回退到原有的 latex-js 方式**（浏览器端渲染，见 [`latex_gen_html`]）。
fn latex_gen_file(mat_str: &str, svg_dir: &Path, chapter_path: &Path, cache_dir: &Path) -> String {
    let mut content = clean_codeblock(mat_str, "```latex tex");

    // 去除所有空行（与 TikZ 一致，保证 pulldown-cmark 不截断 HTML 块）
    content = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    let rel_prefix = crate::utils::relative_svg_prefix(chapter_path);

    // 计算内容 hash，用于缓存与关联中间 PDF 文件（PDF 后处理页面合并）
    let content_hash = crate::tikz::tikz_content_hash(&content);

    #[cfg(feature = "pre-tikz")]
    match crate::tikz::text2svg_file(&content, svg_dir, &rel_prefix, cache_dir,
        &chapter_path.to_string_lossy(), "LaTeX document") {
        Ok(img_tag) => {
            // 与 TikZ 一致：<img> 引用 svg 文件 + data-pdf-hash 容器
            return format!(
                r#"<div data-pdf-hash="{}" align="center" class="diagram-inline">
<style>.diagram-inline img{{width:100%;height:auto;max-width:100%}}@media print{{.diagram-inline img{{width:auto;max-width:100%}}}}</style>
{}
</div>"#,
                content_hash, img_tag
            );
        }
        Err(e) => {
            log::warn!("LaTeX tex 渲染失败，回退到 latex-js 方式: {}", e);
        }
    }

    #[cfg(not(feature = "pre-tikz"))]
    log::warn!("LaTeX tex 渲染不可用（未编译 tectonic 支持），回退到 latex-js 方式");

    // 回退：原有的 latex-js 浏览器端渲染方式
    latex_gen_html(mat_str)
}

/// ===== latex tex (LaTeX 文档，用 <latex-js> 包裹) =====
fn latex_gen_html(mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```latex tex");
    // 消除所有空行，避免 pulldown-cmark 提前结束 HTML 块
    let re = Regex::new(r"\n{2,}").unwrap();
    let content = re.replace_all(&content, "\n");

    let escaped = crate::utils::escape_xml(&content);
    let result = format!(
        r#"<div>
    <div id="CommonMark-latex"></div>
    <latex-js baseURL="https://cdn.jsdelivr.net/npm/latex.js/dist/"><code>
{}
    </code></latex-js>
</div>"#,
        escaped
    );

    result
}

/// ===== latex tikz (TikZ 图片 → tectonic PDF → hayro-svg SVG 内联) =====
fn tikz_gen_file(mat_str: &str, svg_dir: &Path, chapter_path: &Path, cache_dir: &Path) -> String {
    let mut content = clean_codeblock(mat_str, "```latex tikz");

    // 去除所有空行
    content = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    let rel_prefix = crate::utils::relative_svg_prefix(chapter_path);
    log::debug!("TikZ svg_dir: {:?}", svg_dir);

    // 计算内容 hash，用于关联中间 PDF 文件（PDF 后处理页面合并）
    let content_hash = crate::tikz::tikz_content_hash(&content);

    #[cfg(feature = "pre-tikz")]
    match crate::tikz::text2svg_file(&content, svg_dir, &rel_prefix, cache_dir,
        &chapter_path.to_string_lossy(), "TikZ diagram") {
        Ok(img_tag) => {
            // TikZ 用 <img> 引用 svg 文件：svg 作为独立文档加载，样式天然私有
            // （不受 mdbook 主题 CSS 影响），且避免内联 SVG 的字体/样式泄漏问题。
            // 屏幕放大到容器宽、打印保持原始尺寸防溢出。
            return format!(
                r#"<div data-pdf-hash="{}" align="center" class="diagram-inline">
<style>.diagram-inline img{{width:100%;height:auto;max-width:100%}}@media print{{.diagram-inline img{{width:auto;max-width:100%}}}}</style>
{}
</div>"#,
                content_hash, img_tag
            );
        }
        Err(e) => {
            log::warn!("TikZ 渲染失败: {}", e);
        }
    }

    #[cfg(not(feature = "pre-tikz"))]
    log::warn!("TikZ 渲染不可用（未编译 tectonic 支持）");

    // 回退：显示源码
    let re = Regex::new(r"\n{2,}").unwrap();
    let display_content = re.replace_all(&content, "\n");
    format!(
        r#"<div><details><summary>TikZ 渲染失败 (点击展开源码)</summary>
<pre><code>{}</code></pre>
<pre><code>{}</code></pre></details></div>"#,
        "tikz 渲染引擎未编译", display_content,
    )
}

/// ===== pikchr =====
fn pikchr_gen_html(mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```pikchr");
    // pikchr 由独立的 mdbook-pikchr 预处理器处理（pikchr.rs）
    // 这里的 pikchr 处理器是预留，实际由 pikchr.rs 中的 Preprocessor 处理
    // 保留源码显示作为回退
    log::warn!("pikchr 由 pikchr.rs 预处理器处理，echarts 中的 pikchr 处理器不应被调用");
    format!(r#"<pre><code class="language-pikchr">
{}
</code></pre>"#, content)
}

/// ===== typst (Typst 图片 → typst crate → SVG 内联 + PDF 文件) =====
fn typst_gen_file(mat_str: &str, svg_dir: &Path, chapter_path: &Path, cache_dir: &Path) -> String {
    let mut content = clean_codeblock(mat_str, "```typst");

    // 处理 fence info string: "```typst svg" → 去掉 " svg" 等后缀
    // （只保留第一行中 ``` 之后紧跟 typst 的内容，移除额外参数）
    content = clean_typst_fence(&content);

    // 去除所有空行
    content = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    // 计算内容 hash，用于关联中间 PDF 文件（PDF 后处理页面合并）
    let content_hash = crate::typst::typst_content_hash(&content);

    #[cfg(feature = "pre-typst")]
    match crate::typst::text2svg_inline(&content, svg_dir, cache_dir,
        &chapter_path.to_string_lossy()) {
        Ok(svg) => {
            // 内联 SVG + data-pdf-hash 容器；点击放大在 modal 中克隆 SVG
            return format!(
                r#"<div data-pdf-hash="{}" align="center" class="diagram-inline"
style="cursor:zoom-in;" onclick="if(window.miv_openSvgModal){{var s=this.querySelector('svg');if(s)miv_openSvgModal(s);}}">
{}
</div>"#,
                content_hash, svg
            );
        }
        Err(e) => {
            log::warn!("Typst 渲染失败: {}", e);
        }
    }

    #[cfg(not(feature = "pre-typst"))]
    log::warn!("Typst 渲染不可用（未编译 typst 支持）");

    // 回退：显示源码
    let re = Regex::new(r"\n{2,}").unwrap();
    let display_content = re.replace_all(&content, "\n");
    format!(
        r#"<div><details><summary>Typst 渲染失败 (点击展开源码)</summary>
<pre><code>{}</code></pre>
<pre><code>{}</code></pre></details></div>"#,
        "typst 渲染引擎未编译", display_content,
    )
}

/// 清理 typst fence info string，移除 ```typst 之后的额外参数
///
/// 例如 "```typst svg\ntext" → clean_codeblock 后变成 "svg\ntext"
/// 这里只移除已知的渲染器指示关键字（svg/pdf/png/raw/plain），
/// 保留所有其他 typst 源码行。
fn clean_typst_fence(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    // 只移除明确已知的渲染器指示关键字
    let first = lines[0].trim();
    const KNOWN_DIRECTIVES: &[&str] = &["svg", "pdf", "png", "raw", "plain"];
    if KNOWN_DIRECTIVES.contains(&first) {
        lines[1..].join("\n")
    } else {
        content.to_string()
    }
}

/// ===== wavedrom =====
fn wavedrom_gen_html(mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```wavedrom");
    format!(
        r#"<div class="diagram-wavedrom" style="text-align:center;">
<script type="WaveDrom">
{}
</script>
</div>"#,
        content
    )
}

/// 统一的处理入口：供 UnifiedPreprocessor 调用
pub fn process_content(content: &str, _config: Option<&toml::Value>) -> String {
    // 统一入口无法获取 svg_dir/chapter_path，退化为内联 SVG（无缓存）
    let svg_dir = std::path::PathBuf::from("/dev/null/images");
    let chapter_path = std::path::PathBuf::from("index.md");
    let tectonic_cache_dir = std::path::PathBuf::from("/dev/null/Tectonic");
    process_chapter("", content, &svg_dir, &chapter_path, &tectonic_cache_dir)
}

/// 运行 mdbook-echarts 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = ChartPreprocessor;
    crate::utils::run_preprocessor(&pre)
}

/// 收集 book 所有章节内容中引用的 diagram 内容 hash。
///
/// 两类引用来源：
/// 1. `data-pdf-hash="{64hex}"` —— tikz/typst 生成容器上的属性（内联与文件模式都带）；
/// 2. `{64hex}.svg` / `{64hex}.pdf` 文件名引用 —— `<img>` 标签及 md 中手动写出的路径。
///
/// 覆盖了「本次构建实际需要保留」的全部生成物；其余 hash 命名的 `.svg`/`.pdf`
/// 视为上次构建遗留的孤儿缓存。
fn collect_referenced_diagram_hashes(book: &Book) -> HashSet<String> {
    let mut refs = HashSet::new();
    for item in book.iter() {
        if let BookItem::Chapter(chapter) = item {
            collect_referenced_from_content(&chapter.content, &mut refs);
        }
    }
    refs
}

/// 从单个章节内容中提取引用的 diagram hash，并入 `refs`。
fn collect_referenced_from_content(content: &str, refs: &mut HashSet<String>) {
    let re_hash_attr = Regex::new(r#"data-pdf-hash="([0-9a-f]{64})""#).unwrap();
    for cap in re_hash_attr.captures_iter(content) {
        if let Some(m) = cap.get(1) {
            refs.insert(m.as_str().to_string());
        }
    }
    // 文件名引用：{64hex}.svg / {64hex}.pdf / {64hex}.p{i}.svg（覆盖 <img src="..."> 与 md 手写路径）
    let re_file_ref = Regex::new(r"([0-9a-f]{64})(?:\.p\d+)?\.(?:svg|pdf)").unwrap();
    for cap in re_file_ref.captures_iter(content) {
        if let Some(m) = cap.get(1) {
            refs.insert(m.as_str().to_string());
        }
    }
}

/// 清理 `images_dir` 中不再被引用的旧 diagram 缓存文件。
///
/// 只删除「文件名匹配 `{64位hex}.svg` / `{64位hex}.pdf` 且 hash 不在 `referenced`」
/// 的生成物；其他文件（用户手放的非 hash 命名图片等）一律保留。
/// 目录不存在或删除失败时仅记录日志，不阻断构建。
fn cleanup_orphan_diagrams(images_dir: &Path, referenced: &HashSet<String>) {
    if referenced.is_empty() {
        // 本次构建没有任何 diagram 引用（对应 feature 未编译 / 全部渲染失败 /
        // 无 tikz-typst 代码块）时，不能把 src/images 中既有的 hash 命名文件
        // 当作孤儿删除——它们可能是其他来源的生成物。跳过清理，保守保留。
        return;
    }
    let entries = match fs::read_dir(images_dir) {
        Ok(e) => e,
        Err(_) => return, // 目录不存在（本次无任何 tikz/typst 代码块）→ 无需清理
    };
    // 只删除「hash 命名（含分页 {hash}.p{i}.svg）」的生成物
    let re_name = Regex::new(r"^([0-9a-f]{64})(?:\.p\d+)?\.(?:svg|pdf)$").unwrap();

    let mut removed = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Some(cap) = re_name.captures(name_str) else { continue };
        let hash = &cap[1];
        if referenced.contains(hash) {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => {
                removed += 1;
                log::debug!("清理孤儿 diagram 缓存: {}", entry.path().display());
            }
            Err(e) => log::warn!("清理孤儿 diagram 缓存失败 {}: {}", entry.path().display(), e),
        }
    }
    if removed > 0 {
        log::info!("清理 {} 个不再被引用的 diagram 缓存文件", removed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("echarts-cleanup-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn hash_hex(seed: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(seed.as_bytes());
        format!("{:x}", h.finalize())
    }

    #[test]
    fn test_cleanup_orphan_diagrams() {
        let dir = tmpdir();
        let kept_svg = hash_hex("kept-svg");
        let kept_pdf = hash_hex("kept-pdf");
        let orphan_svg = hash_hex("orphan-svg");
        let orphan_pdf = hash_hex("orphan-pdf");

        // 引用的文件
        std::fs::write(dir.join(format!("{}.svg", kept_svg)), "<svg/>").unwrap();
        std::fs::write(dir.join(format!("{}.pdf", kept_pdf)), "pdf").unwrap();
        // 孤儿文件（应被清理）
        std::fs::write(dir.join(format!("{}.svg", orphan_svg)), "<svg/>").unwrap();
        std::fs::write(dir.join(format!("{}.pdf", orphan_pdf)), "pdf").unwrap();
        // 用户手放的非 hash 命名文件（应保留）
        std::fs::write(dir.join("user-diagram.svg"), "<svg/>").unwrap();
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();

        let mut referenced = HashSet::new();
        referenced.insert(kept_svg.clone());
        referenced.insert(kept_pdf.clone());
        cleanup_orphan_diagrams(&dir, &referenced);

        let remaining: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(remaining.contains(&format!("{}.svg", kept_svg)), "引用的 svg 被误删: {:?}", remaining);
        assert!(remaining.contains(&format!("{}.pdf", kept_pdf)), "引用的 pdf 被误删: {:?}", remaining);
        assert!(!remaining.contains(&format!("{}.svg", orphan_svg)), "孤儿 svg 未被清理: {:?}", remaining);
        assert!(!remaining.contains(&format!("{}.pdf", orphan_pdf)), "孤儿 pdf 未被清理: {:?}", remaining);
        assert!(remaining.contains(&"user-diagram.svg".to_string()), "用户文件被误删: {:?}", remaining);
        assert!(remaining.contains(&"notes.txt".to_string()), "用户文件被误删: {:?}", remaining);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_cleanup_missing_dir_is_noop() {
        let dir = std::env::temp_dir().join(format!("echarts-cleanup-none-{}", Uuid::new_v4()));
        let mut referenced = HashSet::new();
        referenced.insert(hash_hex("x"));
        cleanup_orphan_diagrams(&dir, &referenced); // 目录不存在 → 静默跳过
        assert!(!dir.exists());
    }

    #[test]
    fn test_cleanup_empty_referenced_keeps_everything() {
        // 本次构建无任何 diagram 引用（feature 关闭 / 渲染失败）时，
        // 目录中既有的 hash 命名文件必须保留，不能当作孤儿删除。
        let dir = tmpdir();
        let h = hash_hex("existing");
        std::fs::write(dir.join(format!("{}.svg", h)), "<svg/>").unwrap();
        std::fs::write(dir.join(format!("{}.pdf", h)), "pdf").unwrap();

        let referenced = HashSet::new();
        cleanup_orphan_diagrams(&dir, &referenced);

        let remaining: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(remaining.contains(&format!("{}.svg", h)), "空引用集合时不应删除任何文件: {:?}", remaining);
        assert!(remaining.contains(&format!("{}.pdf", h)), "空引用集合时不应删除任何文件: {:?}", remaining);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_collect_referenced_from_content() {
        let a = hash_hex("a");
        let b = hash_hex("b");
        let c = hash_hex("c");
        let content = format!(
            r#"<div data-pdf-hash="{}" class="diagram-inline">inline svg</div>
<img src="../images/{}.svg" alt="tikz">
![fig](images/{}.pdf)
<div data-pdf-hash="{}">x</div>
普通文本里出现 {} 无扩展名不应收集
"#,
            a, b, c, a, c
        );
        let mut refs = HashSet::new();
        collect_referenced_from_content(&content, &mut refs);
        assert_eq!(refs.len(), 3, "应收集 3 个 hash: {:?}", refs);
        assert!(refs.contains(&a));
        assert!(refs.contains(&b));
        assert!(refs.contains(&c));
    }
}
