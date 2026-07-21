use anyhow::Result;
use std::path::Path;
use tectonic::config::PersistentConfig;
use tectonic::driver::{OutputFormat, PassSetting, ProcessingSession, ProcessingSessionBuilder};
use tectonic::status::{plain::PlainStatusBackend, ChatterLevel};

/// Compile a TikZ LaTeX snippet to PDF bytes using tectonic.
///
/// `cache_dir` specifies where tectonic stores the precompiled format (`.fmt`) cache.
pub fn tex_to_pdf(input: &str, cache_dir: &Path) -> Result<Vec<u8>> {
    let content = strip_documentclass(input);

    let source = format!(
        r#"\documentclass[margin=0pt]{{standalone}}
\usepackage{{tikz}}
{}
"#,
        content
    );

    let config = PersistentConfig::open(false)
        .map_err(|e| anyhow::anyhow!("failed to open tectonic config: {:?}", e))?;
    let bundle = config
        .default_bundle(false)
        .map_err(|e| anyhow::anyhow!("failed to get tectonic bundle: {:?}", e))?;

    let mut builder = ProcessingSessionBuilder::default();
    builder
        .output_format(OutputFormat::Pdf)
        .primary_input_buffer(source.as_bytes())
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

fn strip_documentclass(input: &str) -> String {
    let re = regex::Regex::new(r"(?m)^\s*\\documentclass(?:\[.*?\])?\{.*?\}\s*$").unwrap();
    let result = re.replace_all(input, "");
    result
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
