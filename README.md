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
- mdbook-pdf-preview — PDF 引用内嵌预览（📄 占位，滚动到视口后 iframe 内嵌，浏览器原生 PDF viewer 渲染，零 JS 依赖）
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
cargo clean -p mdbook-plugins && cargo build
cargo build --release  # release 模式
# 产物：target/release/mdbook-plugins
```

## 使用

将产物目录加入 `PATH`：
```sh
export PATH="$PATH:$(pwd)/target/release/"
```

> **多 renderer 提速**：mdbook 会对每个 renderer（html、搜索索引等）都跑一遍 preprocessor
> 链。若你的书有多个 renderer，给只服务于 HTML 的 preprocessor 配置
> `renderers = ["html"]`（**注意是复数 `renderers`**，mdbook 读取该键名），可让
> zz-build-search / pdf-preview-assets 等 renderer 跳过这些 preprocessor，避免重复处理：
>
> ```toml
> [preprocessor.katex]
> command = "mdbook-plugins katex"
> renderers = ["html"]
> ```

> **图级内容缓存（serve 反复重建提速）**：echarts（svgbob/bob 大图）、kroki 已内置
> 内容 hash 缓存——**未变化的图直接复用本地 SVG**（存于 `{build_dir}/Svgbob/`、
> `{build_dir}/Kroki/`），只有图内容变化才重新渲染/请求网络。serve 下只改文字章节时，
> 图不再重复渲染（bob 大图从 ~7s 降到 0s）。缓存目录随 `build-dir`，不被 html renderer 清空。

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
    "./assets/pdfviewer/pdf-preview.js",
]
```

完整示例见 `test/book.toml`。

### PDF 预览（mdbook-pdf-preview）

把 `[text](./file.pdf "web-preview")`（链接标题必须为 `web-preview`）替换为内嵌 PDF 预览。
其余 `.pdf` 链接保持普通链接。

**三种渲染模式**（`[preprocessor.pdf-preview] mode` 配置）：

| 模式 | 渲染方式 | 特点 |
|---|---|---|
| `viewer`（默认） | **pdf.js 完整 viewer**（viewer.html 同源加载，内部同源 fetch，**无需 CORS**） | UI 完整（工具栏/缩放/侧边栏/连续滚动/搜索），体验接近浏览器原生 |
| `pdfjs` | pdf.js Canvas（简化 UI，翻页/缩放） | 轻量，viewer 资源缺失时的 fallback |
| `embed` | iframe 内嵌浏览器原生 PDF viewer | **需要服务器返回 CORS 头**（mdbook serve 不返回 → 会触发下载，不推荐 serve 下用） |

**为什么默认 viewer**：Chrome 原生 viewer（embed）在 iframe 中由扩展页**跨源 fetch** PDF，
要求服务器返回 CORS 头；`mdbook serve` 不返回 → 必现下载+空白。pdf.js viewer 是
**同源页面**加载 PDF，无此问题，且 UI 完整。

```toml
[preprocessor.pdf-preview]
command = "mdbook-plugins pdf-preview"
renderer = ["html"]

[output.html]
additional-js = ["./assets/pdfviewer/pdf-preview.js"]   # 前端脚本需自备（见 test/assets/pdfviewer/）

# viewer/pdfjs 模式的本地 pdf.js 资源（html 渲染后复制到输出，不污染 src/）
[output.pdf-preview-assets]
command = "mdbook-plugins pdf-preview-assets"
```

**viewer 模式需要 pdf.js 完整 viewer 资源**（放书目录 `assets/pdfviewer/`，由
`pdf-preview-assets` 渲染器在 html 后复制到输出）：

```sh
# 从 https://github.com/mozilla/pdf.js/releases 下载 pdfjs-6.1.200-dist.zip
mkdir -p assets/pdfviewer
unzip pdfjs-6.1.200-dist.zip -d assets/pdfviewer   # 解出 web/ 与 build/
# 最终结构：
#   assets/pdfviewer/web/viewer.html        ← viewer 页面（可裁剪 locale/cmaps/wasm 等）
#   assets/pdfviewer/build/pdf.mjs          ← viewer.html 引用 ../build/pdf.mjs
#   assets/pdfviewer/build/pdf.worker.mjs   ← worker
```

**资源精简建议**（保持 viewer 可用的最小集，约 5MB）：

- `web/` 可删除：`viewer.mjs.map`（sourcemap）、`compressed.tracemonkey-pldi-09.pdf`（测试文件）、
  `debugger.css/mjs`、`wasm/`、`iccs/`、`locale/`（只留 `en-US` + `zh-CN` 并同步更新
  `locale.json`）、`images/` 中的 `altText_*`/`annotation-*`/`comment-*`/`cursor-editor*`/
  `editor-toolbar*`/`toolbarButton-editor*`（编辑器/注释图标）、`cmaps/`（只留
  `Adobe-{GB1,CNS1,Japan1,Korea1}-*`、`Uni{GB,CNS,JIS,KS}-UTF16-*`、`90ms-RKSJ-*`、
  `90pv-RKSJ-*` 等中文/日文/韩文常用映射；个别 PDF 使用被删 cmap 时字体可能异常，可恢复）
- `build/` 只需 `pdf.mjs` + `pdf.worker.mjs`（pdf.js 核心，必需）

注意：

- `file://` 直接打开构建产物时：Chrome 禁止 file:// 页面的 module script（ESM 需 CORS，
  file:// 是 null origin），pdf.js viewer 无法运行，自动降级为浏览器原生 `<embed>` 渲染
  （同样是完整查看器，可直接查看）。
- **希望 build 产物与 serve 完全同一套（viewer）**：用 http 访问构建产物，例如
  `python -m http.server -d books/html` 后浏览器打开 `http://localhost:8000`，或部署到服务器。
- 首次加载被浏览器扩展/安全功能拦截（204 + 弹窗下载）时，已自动用 cache-buster 重试。
- `pdf-preview.js` 与 mermaid/echarts 的前端资源一样，需自行放进书目录并保持 `additional-js` 路径一致。

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
>
> PDF 渲染耗时主要在资源加载（等待全部图片/样式），与 emoji 处理无关；Windows 下可将构建目录加入
> **Windows Defender 排除项** 缓解实时扫描 IO 开销。`enable-emoji-font` 请保持默认开启：
> 关闭不会省时，且彩色 emoji 系统字体无法嵌入 PDF、会被位图化导致 PDF 体积暴涨（实测 26MB → 96MB）。

## 许可

MIT；内置的 `vendor/pikchr.c` 采用 [Zero-Clause BSD](https://opensource.org/licenses/0BSD)。
