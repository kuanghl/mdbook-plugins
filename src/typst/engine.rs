//! Typst 编译引擎
//!
//! 实现 typst::World trait，将 Typst 源码编译为 SVG 和 PDF。
//!
//! 管线: Typst source → typst::compile() → PagedDocument
//!   → typst_svg::svg_merged() → SVG (供 HTML 使用)
//!   → typst_pdf::pdf() → PDF bytes (供 PDF 渲染器使用)

use anyhow::Result;
use std::path::PathBuf;
use crate::utils::print_status;
use std::sync::LazyLock;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::layout::Abs;
use typst::syntax::{
    package::PackageSpec, FileId, RootedPath, Source, VirtualPath, VirtualRoot,
};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt};
use typst::World;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;
use typst_svg::SvgOptions;

/// Typst 编译环境
struct TypstWorld {
    library: LazyHash<Library>,
    main: FileId,
    source: Source,
}

static SHARED_LIBRARY: LazyLock<LazyHash<Library>> =
    LazyLock::new(|| LazyHash::new(Library::builder().build()));

static SHARED_FONTS: LazyLock<typst_kit::fonts::FontStore> = LazyLock::new(|| {
    let mut store = typst_kit::fonts::FontStore::new();
    // 与 typst CLI 一致：先加载嵌入字体，再加载系统字体
    store.extend(typst_kit::fonts::embedded());
    store.extend(typst_kit::fonts::system());
    store
});

impl TypstWorld {
    fn new(source_text: &str) -> Self {
        let root = VirtualRoot::Project;
        let vpath = VirtualPath::new("main.typ").expect("无效的虚拟路径");
        let rooted = RootedPath::new(root, vpath);
        let main = FileId::new(rooted);
        let source = Source::new(main, source_text.into());
        Self {
            library: SHARED_LIBRARY.clone(),
            main,
            source,
        }
    }

    fn package_root() -> PathBuf {
        if let Ok(dir) = std::env::var("TYPST_PACKAGES") {
            let p = PathBuf::from(dir);
            if p.is_dir() || p.parent().map_or(false, |parent| parent.is_dir()) {
                return p;
            }
        }
        if let Ok(base) = std::env::var("XDG_DATA_HOME") {
            let p = PathBuf::from(base).join("typst").join("packages");
            if p.is_dir() || p.ancestors().any(|a| a.is_dir()) {
                return p;
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        let standard = PathBuf::from(&home).join(".local/share/typst/packages");
        let test_file = standard.join(".write_test");
        if std::fs::create_dir_all(&standard).is_ok()
            && std::fs::write(&test_file, "").is_ok()
        {
            let _ = std::fs::remove_file(&test_file);
            return standard;
        }
        let tmp = PathBuf::from("/tmp/typst-packages");
        log::warn!("~/.local/share/typst/packages 不可写，使用 {:?}", tmp);
        let _ = std::fs::create_dir_all(&tmp);
        tmp
    }

    fn resolve_package(spec: &PackageSpec) -> Option<PathBuf> {
        let root = Self::package_root();
        let package_dir = root
            .join(spec.namespace.as_str())
            .join(spec.name.as_str())
            .join(spec.version.to_string());
        if package_dir.is_dir() {
            return Some(package_dir);
        }
        log::debug!(
            "Typst 包 {}/{}:{} 未在本地找到，正在自动下载...",
            spec.namespace, spec.name, spec.version
        );
        print_status(&format!(
            "Downloading Typst package: {}/{} v{}",
            spec.namespace, spec.name, spec.version
        ));
        match Self::download_package(spec, &root) {
            Ok(dir) => {
                print_status(&format!(
                    "Typst package downloaded: {}/{} v{}",
                    spec.namespace, spec.name, spec.version
                ));
                log::debug!(
                    "Typst 包 {}/{}:{} 下载完成到 {:?}",
                    spec.namespace, spec.name, spec.version, dir
                );
                Some(dir)
            }
            Err(e) => {
                log::error!(
                    "Typst 包 {}/{}:{} 自动下载失败: {}",
                    spec.namespace, spec.name, spec.version, e
                );
                None
            }
        }
    }

    /// 从 GitHub raw 下载 Typst 包（先试官方 registry，失败则走国内 ghproxy 镜像）
    fn download_package(spec: &PackageSpec, root: &PathBuf) -> Result<PathBuf> {
        let version_str = spec.version.to_string();
        let package_dir = root
            .join(spec.namespace.as_str())
            .join(spec.name.as_str())
            .join(&version_str);

        if let Some(parent) = package_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("无法创建目录 {:?}: {}", parent, e))?;
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent("mdbook-plugins/0.1")
            .build()
            .map_err(|e| anyhow::anyhow!("创建 HTTP 客户端失败: {}", e))?;

        // 先尝试官方 registry（tar.gz 单文件，最快捷）
        let registry_url = format!(
            "https://packages.typst.org/{}/{}-{}.tar.gz",
            spec.namespace, spec.name, version_str
        );
        log::debug!("尝试官方 registry: {}", registry_url);

        if let Ok(resp) = client.get(&registry_url).send() {
            if resp.status().is_success() {
                return Self::extract_tar_gz(resp, &package_dir, spec, &version_str);
            }
        }

        // 官方 registry 不可用，尝试国内镜像（ghproxy → GitHub raw）
        log::warn!("官方 registry 不可用，切换国内镜像源...");
        let files = Self::list_package_files(spec, &version_str, &client)?;
        log::debug!("找到 {} 个文件，开始下载...", files.len());
        print_status(&format!(
            "Downloading Typst package files: {} files",
            files.len()
        ));

        let base_url = "https://ghproxy.net/https://raw.githubusercontent.com/typst/packages/main";
        let prefix = format!("packages/{}/{}/{}", spec.namespace, spec.name, version_str);

        for (idx, rel_path) in files.iter().enumerate() {
            let target_path = package_dir.join(rel_path);
            if let Some(parent) = target_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let file_url = format!("{}/{}/{}", base_url, prefix, rel_path);
            log::debug!("  [{}/{}] {}", idx + 1, files.len(), rel_path);

            if let Ok(resp) = client.get(&file_url).send() {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes() {
                        let _ = std::fs::write(&target_path, &bytes);
                    }
                }
            }
        }

        log::debug!("包 {} v{} 下载完成", spec.name, version_str);
        Ok(package_dir)
    }

    /// 通过 GitHub Contents API 列出包的必要文件（.typ + typst.toml）
    fn list_package_files(spec: &PackageSpec, version_str: &str, client: &reqwest::blocking::Client) -> Result<Vec<String>> {
        let api_url = format!(
            "https://api.github.com/repos/typst/packages/contents/packages/{}/{}/{}",
            spec.namespace, spec.name, version_str
        );
        let text = client.get(&api_url).send()
            .map_err(|e| anyhow::anyhow!("获取包目录失败: {}", e))?
            .text()
            .map_err(|e| anyhow::anyhow!("读取响应失败: {}", e))?;
        let items: Vec<serde_json::Value> = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("解析包目录失败: {}", e))?;

        let mut files = Vec::new();
        for item in &items {
            let name = item["name"].as_str().unwrap_or("");
            let type_ = item["type"].as_str().unwrap_or("");
            match type_ {
                "file" => {
                    if name.ends_with(".typ") || name == "typst.toml" {
                        files.push(name.to_string());
                    }
                }
                "dir" => {
                    let sub_url = format!("{}/{}", api_url, name);
                    if let Ok(sub_resp) = client.get(&sub_url).send() {
                        let sub_text = sub_resp.text().unwrap_or_default();
                        if let Ok(sub_items) = serde_json::from_str::<Vec<serde_json::Value>>(&sub_text) {
                            for sub in &sub_items {
                                if let Some(sub_name) = sub["name"].as_str() {
                                    if sub["type"].as_str() == Some("file")
                                        && (sub_name.ends_with(".typ") || sub_name.ends_with(".toml"))
                                    {
                                        files.push(format!("{}/{}", name, sub_name));
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(files)
    }

    /// 解压官方 registry 的 tar.gz
    fn extract_tar_gz(
        response: reqwest::blocking::Response,
        package_dir: &PathBuf,
        spec: &PackageSpec,
        version_str: &str,
    ) -> Result<PathBuf> {
        let total_size = response.content_length().unwrap_or(0);
        log::debug!("包大小: {} bytes", total_size);

        let bytes = response.bytes()
            .map_err(|e| anyhow::anyhow!("读取包数据失败: {}", e))?;
        eprintln!("DEBUG: 已下载 {} bytes, 准备解压到 {:?}", bytes.len(), package_dir);

        // 确保目标目录存在
        std::fs::create_dir_all(package_dir)
            .map_err(|e| anyhow::anyhow!("创建目录 {:?} 失败: {}", package_dir, e))?;

        let decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut archive = tar::Archive::new(decoder);
        let prefix = format!("{}-{}", spec.name, version_str);
        eprintln!("DEBUG: tar strip prefix: {:?}", prefix);

        let mut file_count = 0;
        for entry in archive.entries().map_err(|e| anyhow::anyhow!("读取 tar 失败: {}", e))? {
            let mut entry = entry.map_err(|e| anyhow::anyhow!("读取 tar 条目失败: {}", e))?;
            let path = entry.path().map_err(|e| anyhow::anyhow!("读取路径失败: {}", e))?;
            let path_str = path.to_string_lossy().to_string();
            let relative = path.strip_prefix(&prefix).unwrap_or(&path);
            let target = package_dir.join(relative);
            
            eprintln!("DEBUG tar entry: {} -> {}", path_str, target.display());
            
            if entry.header().entry_type().is_dir() {
                let _ = std::fs::create_dir_all(&target);
            } else {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                entry.unpack(&target)
                    .map_err(|e| anyhow::anyhow!("解压 {:?} 失败: {}", target, e))?;
                file_count += 1;
            }
        }
        eprintln!("DEBUG: 解压完成: {} 个文件", file_count);
        log::debug!("包 {} v{} 下载并解压完成 ({} 文件)", spec.name, version_str, file_count);
        Ok(package_dir.clone())
    }
}

impl World for TypstWorld {
    fn library(&self) -> &LazyHash<Library> { &self.library }
    fn book(&self) -> &LazyHash<FontBook> { SHARED_FONTS.book() }
    fn main(&self) -> FileId { self.main }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.source.clone())
        } else if let VirtualRoot::Package(spec) = id.root() {
            let pkg_dir = Self::resolve_package(spec)
                .ok_or_else(|| FileError::NotFound(PathBuf::new()))?;
            let file_path = pkg_dir.join(id.vpath().get_without_slash());
            let content = std::fs::read_to_string(&file_path)
                .map_err(|_| FileError::NotFound(file_path))?;
            Ok(Source::new(id, content.into()))
        } else {
            Err(FileError::NotFound(PathBuf::new()))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main {
            let text = self.source.text().as_bytes().to_vec();
            Ok(Bytes::new(text))
        } else if let VirtualRoot::Package(spec) = id.root() {
            let pkg_dir = Self::resolve_package(spec)
                .ok_or_else(|| FileError::NotFound(PathBuf::new()))?;
            let file_path = pkg_dir.join(id.vpath().get_without_slash());
            let data = std::fs::read(&file_path)
                .map_err(|_| FileError::NotFound(file_path))?;
            Ok(Bytes::new(data))
        } else {
            Err(FileError::NotFound(PathBuf::new()))
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        SHARED_FONTS.font(index)
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        Datetime::from_ymd_hms(2026, 6, 1, 0, 0, 0)
    }
}

pub fn typst_to_svg(source: &str) -> Result<String> {
    let world = TypstWorld::new(source);
    let warned = typst::compile::<PagedDocument>(&world);

    // 输出警告详情
    for w in &warned.warnings {
        log::warn!("Typst 编译警告: {} (span={:?})", w.message, w.span);
    }

    let document = warned.output.map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.message.to_string()).collect();
        anyhow::anyhow!("Typst 编译失败: {}", msgs.join("; "))
    })?;

    // 输出页面信息
    let pages = document.pages();
    log::debug!("Typst 编译完成: {} 页, {} 警告", pages.len(), warned.warnings.len());
    print_status(&format!(
        "Typst compilation: {} pages, {} warnings",
        pages.len(),
        warned.warnings.len()
    ));

    let options = SvgOptions::default();
    let gap = Abs::zero();
    let svg = typst_svg::svg_merged(&document, &options, gap);

    Ok(svg)
}

pub fn typst_to_pdf(source: &str) -> Result<Vec<u8>> {
    let world = TypstWorld::new(source);
    let warned = typst::compile::<PagedDocument>(&world);

    for w in &warned.warnings {
        log::warn!("Typst 编译警告: {} (span={:?})", w.message, w.span);
    }

    let document = warned.output.map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.message.to_string()).collect();
        anyhow::anyhow!("Typst 编译失败: {}", msgs.join("; "))
    })?;

    log::debug!("Typst PDF 输出: {} 页", document.pages().len());
    print_status(&format!(
        "Typst PDF output: {} pages",
        document.pages().len()
    ));

    let options = PdfOptions::default();
    let pdf = typst_pdf::pdf(&document, &options).map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.message.to_string()).collect();
        anyhow::anyhow!("Typst PDF 导出失败: {}", msgs.join("; "))
    })?;

    Ok(pdf)
}
