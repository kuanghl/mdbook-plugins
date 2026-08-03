# mdbook-plugins

单二进制多插件支持的 [mdbook](https://github.com/rust-lang/mdBook) 插件集合：将 **20 个独立插件**（15 个预处理器 + 5 个渲染器）整合为一个二进制，通过 `command` 字段路由分发，支持 Cargo features 选择性构建。

## 插件列表

**预处理器（15）**

- [mdbook-admonish](https://github.com/tommilligan/mdbook-admonish.git) — Material Design 提示框（笔记/警告/危险等）
- [mdbook-alerts](https://github.com/lambdalisue/rs-mdbook-alerts.git) — GitHub 风格 Alert 语法
- [mdbook-echarts](https://github.com/zhuangbiaowei/mdbook-echarts.git) — 统一图表处理（ECharts / Svgbob / Bytefield / LaTeX / TikZ / Pikchr / Typst / WaveDrom）
- [mdbook-emojicodes](https://github.com/blyxyas/mdbook-emojicodes.git) — Emoji shortcode 替换
- [mdbook-embedify](https://github.com/MR-Addict/mdbook-embedify.git) — 嵌入式内容（YouTube / CodePen / Giscus 等）
- mdbook-image-viewer — 图片点击放大（模态框，支持拖拽/滚轮缩放/触控）
- [mdbook-katex](https://github.com/lzanini/mdbook-katex.git) — LaTeX 数学公式服务端预渲染（KaTeX）
- [mdbook-kroki-preprocessor](https://github.com/JoelCourtney/mdbook-kroki-preprocessor.git) — Kroki 远程渲染（Graphviz / PlantUML / D2 等）
- [mdbook-langtabs](https://github.com/nx10/mdbook-langtabs.git) — 多语言标签页
- [mdbook-mermaid](https://github.com/badboy/mdbook-mermaid.git) — Mermaid 图表占位
- mdbook-pdf-preview — PDF 引用内嵌预览（📄 占位，点击 Canvas 渲染，基于 pdf.js）
- [mdbook-pikchr](https://github.com/podsvirov/mdbook-pikchr.git) — Pikchr 图 → 内联 SVG（内置 C 库）
- [mdbook-svgbob](https://github.com/boozook/mdbook-svgbob.git) — ASCII art → SVG
- [mdbook-toc](https://github.com/badboy/mdbook-toc.git) — 自动生成章节目录
- [mdbook-wavedrom-rs](https://github.com/coastalwhite/wavedrom-rs.git) — 时序图占位

**渲染器（5）**

- [mdbook-asciidoc](https://github.com/daviddrysdale/mdbook-asciidoc.git) — 输出 AsciiDoc 格式
- mdbook-build-search — 构建中文 bigram 搜索索引
- [mdbook-linkcheck](https://github.com/Michael-F-Bryan/mdbook-linkcheck.git) — 检查书中所有 Markdown 链接
- mdbook-office — 输出 DOCX / XLSX / PPTX（依赖 Chrome/Chromium）
- [mdbook-pdf](https://github.com/HollowMan6/mdbook-pdf.git) — PDF 生成（轻量 CDP + CLI 双后端）

## 安装

### 预编译二进制（推荐，免编译）

打 tag（`v*`）时 GitHub Actions 自动在 **Windows / Linux / macOS** 三个平台编译全功能版（含 TikZ/Typst），上传到 [GitHub Releases](https://github.com/kuanghl/mdbook-plugins/releases)。

```powershell
# 方式一：cargo-binstall（自动匹配平台下载，零编译）
cargo install cargo-binstall
cargo binstall mdbook-plugins

# 方式二：手动下载 Releases 中的 mdbook-plugins-<target>.zip/.tar.gz
# （target：x86_64-pc-windows-msvc / x86_64-unknown-linux-gnu / aarch64-apple-darwin）
# 解压后把 mdbook-plugins(.exe) 所在目录加入 PATH
```

### 源码编译

```sh
cargo install mdbook-plugins
mdbook-plugins --help
mdbook-plugins --version
```

> Windows 源码编译需先处理 tectonic 系统库，见下方「Windows 安装说明」。

## 构建

```sh
cargo build            # debug 模式（更快，但体积大）
cargo build --release  # release 模式
# 产物：target/release/mdbook-plugins
```

## 使用

将产物目录加入 `PATH`：

```sh
export PATH="$PATH:$(pwd)/target/release/"
```

1. 在 `book.toml` 中通过 `command` 字段注册插件（无需符号链接）：

```toml
[preprocessor.katex]
command = "mdbook-plugins katex"

[output.pdf]
command = "mdbook-plugins pdf"
```

二进制需在 `PATH` 中，或使用绝对路径：`command = "/path/to/mdbook-plugins pdf"`。

2. 编写 `src/SUMMARY.md` 与 `.md` 文件。
3. 执行 `mdbook build` / `mdbook serve --open`。

查看帮助与版本：

```sh
mdbook-plugins --help      # 用法与当前启用的插件列表
mdbook-plugins --version   # 版本号（或 -V）
```

配置示例（需设置 `use-default-preprocessors = false`）：

```toml
[book]
title = "我的文档"
authors = ["me"]
language = "zh"
src = "src"

[build]
use-default-preprocessors = false

[preprocessor.alerts]

[preprocessor.katex]
after = ["links"]
no-css = true
include-src = true

[preprocessor.echarts]
after = ["katex"]

[preprocessor.toc]
renderer = ["html"]

[output.html]
curly-quotes = true
mathjax-support = true
additional-css = ["katex.min.css", "./theme/mdbook-admonish.css"]
additional-js = [
    "./assets/mermaid/mermaid.min.js",
    "./assets/echarts/echarts.min.js",
]
```

完整示例见 `test/book.toml`。

## 选择性构建

通过 Cargo features 按需编译，减小体积：

```sh
# 仅构建 TOC + KaTeX + Pikchr
cargo build --release --no-default-features \
    --features "pre-toc,pre-katex,pre-pikchr"

# 完整构建（默认，不含 Office）
cargo build --release

# 额外启用 Office 渲染器（DOCX / XLSX / PPTX）
cargo build --release --features ren-office
```

| Feature | 对应插件 | 说明 |
|---------|---------|------|
| `pre-*` | 各预处理器 | 轻量，体积影响小 |
| `pre-tikz` | TikZ/LaTeX 渲染（tectonic + hayro-svg） | 🔴 **极大**（~几十 MB） |
| `pre-typst` | Typst 图表渲染（typst 引擎） | 🔴 **大** |
| `ren-*` | 各渲染器 | 除 `ren-office` 外默认启用 |
| `pre-pdf-cdp-heavy` | PDF 重型 CDP 后端（chromiumoxide） | 🟡 **中等**（默认轻量 CDP） |

> 不需要 TikZ/LaTeX 时关闭 `pre-tikz` 可省去 tectonic 引擎（最大体积贡献者）。

## 本地测试

```sh
cd test
export PATH="$PATH:$(pwd)/../target/release/"
mdbook build
mdbook serve --open
# MDBOOK_LOG=html5ever=off mdbook build   # 禁用 html5ever 日志
```

- `cargo test`：各模块单元测试（KaTeX / ECharts / Mermaid / PDF / TikZ / Typst 等）
- `mdbook build`：完整书籍构建，HTML / PDF 输出到 `test/books/`（最小验证集见 `test-mini/`）

## 环境依赖

- **Rust**（edition 2021）
- **Chrome/Chromium**（可选）：用于 Office 和 PDF 渲染

公式渲染使用内置 `katex-rs`（纯 Rust，无需 Node.js）。

### Windows 安装说明

默认构建（`all` features）包含 `pre-tikz`（tectonic TeX 引擎），其编译需要 **pkg-config 与 libpng** 系统库。
Windows 上未安装时会报错：`The pkg-config command could not be found`（来自 `tectonic_bridge_png`）。

```sh
git clone https://github.com/microsoft/vcpkg
.\vcpkg\bootstrap-vcpkg.bat
# 统一 :x64-windows-static-md（静态链接），编译产物为单个 exe、无 vcpkg dll 依赖
.\vcpkg\vcpkg install libpng:x64-windows-static-md freetype:x64-windows-static-md fontconfig:x64-windows-static-md graphite2:x64-windows-static-md harfbuzz:x64-windows-static-md icu:x64-windows-static-md

# 永久设置环境变量（用户级，写入注册表；VCPKG_ROOT 请替换为实际路径）
setx TECTONIC_DEP_BACKEND "vcpkg"
setx VCPKG_ROOT "C:\path\to\vcpkg"
setx VCPKGRS_TRIPLET "x64-windows-static-md"

# setx 对当前终端不生效，请新开一个终端后执行
cargo install mdbook-plugins
```

## 注意事项

> 将 `katex.min.css` 放在书籍根目录，否则公式格式不正确。
>
> `mdbook` 保持 0.4.36 版本，否则格式不正确。
>
> Windows 上 `mdbook-katex` 使用 `x86_64-pc-windows-gnu.zip` 版本，否则 KaTeX 格式不正确。

## 许可

MIT；内置的 `vendor/pikchr.c` 采用 [Zero-Clause BSD](https://opensource.org/licenses/0BSD)。
