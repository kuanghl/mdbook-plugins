use anyhow::Result;
use std::path::Path;
use tectonic::config::PersistentConfig;
use tectonic::driver::{OutputFormat, PassSetting, ProcessingSession, ProcessingSessionBuilder};
use tectonic::status::{plain::PlainStatusBackend, ChatterLevel};

/// Compile a TikZ LaTeX snippet to PDF bytes using tectonic.
///
/// `cache_dir` specifies where tectonic stores the precompiled format (`.fmt`) cache.
pub fn tex_to_pdf(input: &str, cache_dir: &Path) -> Result<Vec<u8>> {
    // mdbook 每次构建会清空 build-dir，导致 tectonic format 缓存目录被删除；
    // tectonic 不会自动创建该目录，这里确保其存在（"cannot write format file" 修复）
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| anyhow::anyhow!("failed to create tectonic format cache dir {:?}: {}", cache_dir, e))?;

    let content = build_source(input);

    let config = PersistentConfig::open(false)
        .map_err(|e| anyhow::anyhow!("failed to open tectonic config: {:?}", e))?;
    let bundle = config
        .default_bundle(false)
        .map_err(|e| anyhow::anyhow!("failed to get tectonic bundle: {:?}", e))?;

    let mut builder = ProcessingSessionBuilder::default();
    builder
        .output_format(OutputFormat::Pdf)
        .primary_input_buffer(content.as_bytes())
        .tex_input_name("tikzinput.tex")
        .format_name("latex")
        .format_cache_path(cache_dir)
        .do_not_write_output_files()
        .bundle(bundle)
        .pass(PassSetting::Default);

    // Use PlainStatusBackend so users can see tectonic download/compilation progress on stderr
    let mut status = PlainStatusBackend::new(ChatterLevel::Normal);
    status.always_stderr(true);
    let mut session: ProcessingSession = builder
        .create(&mut status)
        .map_err(|e| anyhow::anyhow!("failed to create tectonic session: {:?}", e))?;

    session
        .run(&mut status)
        .map_err(|e| anyhow::anyhow!("tectonic compilation failed: {:?}", e))?;

    let files = session.into_file_data();
    let pdf = files
        .get("tikzinput.pdf")
        .ok_or_else(|| anyhow::anyhow!("tectonic did not produce a PDF output file"))
        .map(|info| info.data.clone())?;

    Ok(pdf)
}

/// 组装最终的 LaTeX 源码。
///
/// 模板固定提供 `standalone` 文档类与 tikz 宏包。输入有两种形态：
///
/// 1. **完整 standalone 文档**（含 `\begin{document}`，如
///    `test/7.latex_pictures.md` 中的写法）：剥离用户自己的 `\documentclass`
///    行后原样插入——用户内容自带的 `\begin{document}`/`\end{document}`
///    充当文档环境，`\usepackage`/`\usetikzlibrary` 等 preamble 命令自然位于
///    document 之前，无需重新包装。
///
/// 2. **裸 TikZ 片段**（只有 `\begin{tikzpicture}`）：把内容中的 preamble 命令
///    （`\usepackage`、`\usetikzlibrary`、`\tikzset`、`\newcommand` 等）提取到
///    `\begin{document}` 之前，其余主体包裹进 document 环境。
fn build_source(input: &str) -> String {
    let header = "\\documentclass[margin=0pt]{standalone}\n\\usepackage{tikz}\n";
    let stripped = strip_documentclass(input);

    if stripped.contains("\\begin{document}") {
        // 完整 standalone 文档：直接使用内容自带的 document 环境
        format!("{header}{stripped}\n")
    } else {
        let (preamble, body) = split_preamble(&stripped);
        format!("{header}{preamble}\\begin{{document}}\n{body}\\end{{document}}\n")
    }
}

/// 从裸 TikZ 片段中分离 preamble 命令（返回 (preamble, body)）。
///
/// 逐行扫描；识别 preamble 命令起始行后，持续吞入后续行直到花括号平衡且行尾
/// 无 `%` 续行（`\tikzset` 跨行块、`\newcommand` 带 `%` 续行等都能正确处理）。
fn split_preamble(content: &str) -> (String, String) {
    const PREAMBLE_CMDS: &[&str] = &[
        "\\usepackage",
        "\\RequirePackage",
        "\\usetikzlibrary",
        "\\pgfplotsset",
        "\\tikzset",
        "\\definecolor",
        "\\colorlet",
        "\\newcommand",
        "\\renewcommand",
        "\\providecommand",
        "\\newenvironment",
        "\\renewenvironment",
        "\\newlength",
        "\\newsavebox",
        "\\newcount",
        "\\newdimen",
        "\\pgfdeclarelayer",
        "\\pgfsetlayers",
        "\\input",
        "\\include",
        "\\graphicspath",
        "\\addbibresource",
    ];

    let mut preamble = String::new();
    let mut body = String::new();
    let mut in_block = false;
    let mut in_env = false; // 已进入 \begin{...} 环境（之后不再提取 preamble）
    let mut brace_depth = 0i32;

    for line in content.lines() {
        let trimmed = line.trim_start();
        // 进入 \begin{...} 环境后，之后的 \tikzset/\newcommand 是图内局部设置，
        // 必须留在正文（提到 document 前会变全局、引用图中节点名会编译失败）
        if !in_env && trimmed.starts_with("\\begin{") {
            in_env = true;
        }
        if in_env {
            body.push_str(line);
            body.push('\n');
            continue;
        }
        let starts_preamble = PREAMBLE_CMDS.iter().any(|c| trimmed.starts_with(c));
        if starts_preamble || in_block {
            if starts_preamble && !in_block {
                in_block = true;
                brace_depth = 0;
            }
            preamble.push_str(line);
            preamble.push('\n');
            brace_depth += count_braces(line);
            let continues = line.trim_end().ends_with('%'); // TeX 行尾 % 表示续行
            let has_open_brace = line.contains('{');
            if brace_depth <= 0 && !continues && has_open_brace {
                in_block = false;
                brace_depth = 0;
            }
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    (preamble, body)
}

/// 统计一行中 `{`/`}` 的净深度
fn count_braces(line: &str) -> i32 {
    let mut depth = 0i32;
    for c in line.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn strip_documentclass(input: &str) -> String {
    let re = regex::Regex::new(r"(?m)^\s*\\documentclass(?:\[.*?\])?\{.*?\}\s*$").unwrap();
    let result = re.replace_all(input, "");
    result
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
