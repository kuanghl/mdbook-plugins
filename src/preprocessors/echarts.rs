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
//! - ```typst      → <details> 折叠 + <img> SVG
//! - ```wavedrom   → WaveDrom script

use mdbook_core::book::{Book, BookItem};
use mdbook_core::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use regex::Regex;
use std::sync::atomic::{AtomicI32, Ordering};
use uuid::Uuid;

static PICTUREINDEX: AtomicI32 = AtomicI32::new(0);

pub struct ChartPreprocessor;

impl Preprocessor for ChartPreprocessor {
    fn name(&self) -> &str {
        "mdbook-echarts"
    }

    fn supports_renderer(&self, renderer: &str) -> mdbook_core::errors::Result<bool> {
        Ok(renderer == "html")
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        book.for_each_mut(|item: &mut BookItem| {
            PICTUREINDEX.store(0, Ordering::SeqCst);
            if let BookItem::Chapter(ref mut chapter) = item {
                chapter.content = process_chapter(&chapter.name, &chapter.content);
            }
        });
        Ok(book)
    }
}

fn process_chapter(name: &str, content: &str) -> String {
    let chapter_name = name.replace(['/', '\\', ' '], "_");
    let chapter_alt = name.split('/').last().unwrap_or(name); // for alt text, keep original chars
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

    // 4) ```latex tex (LaTeX 文档)
    let re = Regex::new(r"```latex tex((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = latex_gen_html(mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 5) ```latex tikz (TikZ 图片 → SVG)
    let re = Regex::new(r"```latex tikz((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = tikz_gen_file(&chapter_name, &chapter_alt, mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 6) ```pikchr
    let re = Regex::new(r"```pikchr((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = pikchr_gen_html(mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
    }

    // 7) ```typst
    let re = Regex::new(r"```typst((.*\n)+?)?```").unwrap();
    for mat in re.find_iter(s.clone().as_str()) {
        let buf = typst_gen_file(&chapter_name, &chapter_alt, mat.as_str());
        s = s.replace(mat.as_str(), buf.as_str());
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
    format!(
        r#"<div>
    <div id="CommonMark-bytefield" style="text-align: center;">
    <pre tabindex="0"><code class="language-bytefield" data-lang="bytefield">
{}
    </code></pre>
    </div>
</div>"#,
        content
    )
}

/// ===== latex tex (LaTeX 文档，用 <latex-js> 包裹) =====
fn latex_gen_html(mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```latex tex");
    // 消除所有空行，避免 pulldown-cmark 提前结束 HTML 块
    let re = Regex::new(r"\n{2,}").unwrap();
    let content = re.replace_all(&content, "\n");

    let result = format!(
        r#"<div>
    <div id="CommonMark-latex"></div>
    <latex-js baseURL="https://cdn.jsdelivr.net/npm/latex.js/dist/"><code>
{}
    </code></latex-js>
</div>"#,
        content
    );

    result
}

/// ===== latex tikz (TikZ 图片 → SVG) =====
fn tikz_gen_file(chapter_name: &str, chapter_alt: &str, mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```latex tikz");

    // 提取标题（从 % 注释中）
    let re_title = Regex::new(r"^\s*%+\s*([[:word:]]+)").unwrap();
    let title = content.lines().find_map(|line| {
        re_title.captures(line)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }).unwrap_or_else(|| "samples".to_string());

    let idx = PICTUREINDEX.fetch_add(1, Ordering::SeqCst);
    let svgname = format!("{}_{}.svg", title, idx);

    // 消除多余空行
    let re = Regex::new(r"\n{3,}").unwrap();
    let display_content = re.replace_all(&content, "\n\n");

    format!(
        r#"<div><details><summary>{svgfile}</summary>
<div id="CommonMark-latex tikz"></div>

<pre><code class="language-latex tikz">
{content}
</code></pre></details></div>
<div align=center>
<img src="./../images/{chapter}/{svg}" alt="{chapter_alt}" class="miv_mdbook-image-viewer" onclick="miv_openModal(this.src)">
</div>"#,
        svgfile = svgname.replace(".svg", ".tex"),
        content = display_content,
        chapter = chapter_name,
        chapter_alt = chapter_alt,
        svg = svgname,
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

/// ===== typst =====
fn typst_gen_file(chapter_name: &str, _chapter_alt: &str, mat_str: &str) -> String {
    let content = clean_codeblock(mat_str, "```typst");

    // 提取标题（从 // 注释中）
    let re_title = Regex::new(r"^\s*//+\s*([[:word:]]+)").unwrap();
    let title = content.lines().find_map(|line| {
        re_title.captures(line)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }).unwrap_or_else(|| "samples".to_string());

    let idx = PICTUREINDEX.fetch_add(1, Ordering::SeqCst);
    let svgname = format!("{}_{}.svg", title, idx);

    // 消除多余空行
    let re = Regex::new(r"\n{3,}").unwrap();
    let display_content = re.replace_all(&content, "\n\n");

    format!(
        r#"<div><details><summary>{svgfile}</summary>
<div id="CommonMark-typst"></div>

<pre><code class="language-typst">
{content}
</code></pre></details></div>
<div align=center>
<img src="./../images/{chapter}/{svg}" alt="{chapter}" class="miv_mdbook-image-viewer" onclick="miv_openModal(this.src)">
</div>"#,
        svgfile = svgname,
        content = display_content,
        chapter = chapter_name,
        svg = svgname,
    )
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
    process_chapter("", content)
}

/// 运行 mdbook-echarts 预处理器
pub fn run() -> anyhow::Result<()> {
    let pre = ChartPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
