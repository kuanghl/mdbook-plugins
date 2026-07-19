# mdbook-plugins

单二进制多插件支持的 [mdbook](https://github.com/rust-lang/mdBook) 插件集合。

将 17 个独立插件（13 个预处理器 + 4 个渲染器）整合为**一个二进制**，通过 `argv[0]` 符号链接路由分发。Release 体积约 **7.5 MB**（LTO + strip），原始独立二进制总大小约 **115 MB**，节省约 **94%**。

## 插件列表

### 预处理器

| 插件 | 功能 | 触发语法 |
|------|------|---------|
| **mdbook-admonish** | Material Design 提示框（笔记/警告/危险等） | ` ```admonish <type> ` |
| **mdbook-alerts** | GitHub 风格 Alert 语法 | `> [!NOTE]` / `> [!WARNING]` |
| **mdbook-echarts** | 统一图表处理（ECharts / Svgbob / Bytefield / LaTeX / Pikchr / Typst / WaveDrom） | ` ```echarts ` / ` ```bob ` / ` ```bytefield ` 等 |
| **mdbook-emojicodes** | Emoji shortcode 替换 | `:smile:` → 😄 |
| **mdbook-embedify** | 嵌入式内容（YouTube / CodePen / Giscus 等） | `{% youtube ... %}` |
| **mdbook-image-viewer** | 图片点击放大（模态框，支持拖拽/滚轮缩放/触控） | `![alt](path)` |
| **mdbook-katex** | LaTeX 数学公式服务端预渲染（KaTeX） | `$...$` / `$$...$$` |
| **mdbook-kroki-preprocessor** | Kroki 远程渲染（Graphviz / PlantUML / D2 等） | ` ```kroki-<type> ` |
| **mdbook-langtabs** | 多语言标签页 | `<!-- langtabs-start -->` |
| **mdbook-mermaid** | Mermaid 图表占位 | ` ```mermaid ` |
| **mdbook-pikchr** | Pikchr 图 → 内联 SVG（内置 C 库） | ` ```pikchr ` |
| **mdbook-svgbob** | ASCII art → SVG | ` ```bob ` |
| **mdbook-toc** | 自动生成章节目录 | `<!-- toc -->` |
| **mdbook-wavedrom-rs** | 时序图占位 | ` ```wavedrom ` |

### 渲染器

| 插件 | 功能 |
|------|------|
| **mdbook-asciidoc** | 输出 AsciiDoc 格式 |
| **mdbook-linkcheck** | 检查书中所有 Markdown 链接 |
| **mdbook-office** | 输出 DOCX / XLSX / PPTX（依赖 Chrome/Chromium） |
| **mdbook-pdf** | PDF 生成（Chrome CDP + CLI 双后端） |

## 快速开始

### 构建

```bash
git clone <repo-url>
cd mdbook-plugins

# 构建（Release 模式）
cargo build --release
cp target/release/mdbook-plugins test/bin/

# 测试
cd test-mini
export PATH="$PATH:$(pwd)/../target/release/"
mdbook build

cd test
export PATH="$PATH:$(pwd)/../target/release/"
mdbook build
```

构建完成后，二进制自动部署到 `test/bin/mdbook-plugins`（通过 `build.rs`）。如无 Chrome/Chromium，可用轻量方案：

```bash
# 1. 添加第三方 PPA 源
sudo add-apt-repository ppa:xtradeb/apps -y
# sudo add-apt-repository --remove ppa:xtradeb/apps

# 2. 更新软件包列表
sudo apt update

# 3. 安装传统的 .deb 版 Chromium
sudo apt install chromium

# 或者
# 下载 chrome的安装包
wget https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb
# 安装 Chrome
sudo dpkg -i google-chrome-stable_current_amd64.deb

# 移除软件包并删除配置文件
sudo apt purge chromium

# 清理不再需要的依赖包
sudo apt autoremove
```

### 部署

所有插件（预处理和渲染器）统一通过 `book.toml` 的 `command` 字段调用，无需符号链接：

```toml
[preprocessor.katex]
command = "mdbook-plugins katex"

[output.pdf]
command = "mdbook-plugins pdf"
```

二进制路径需在 `PATH` 中，或使用绝对路径：`command = "/path/to/mdbook-plugins pdf"`。

### 运行测试

```bash
cd test

# 首次需安装 KaTeX（用于公式预渲染，可选）
npm install katex@0.12.0

# 执行构建
PATH="bin:$PATH" mdbook build
```

测试包含：
- 12 个预处理器插件的 `supports` 协议测试
- 路由正确性测试
- 完整 mdbook 构建 + PDF 生成测试

## 配置示例

在 `book.toml` 中配置插件（需设置 `use-default-preprocessors = false`）：

```toml
[book]
title = "我的文档"
authors = ["me"]
language = "zh"
src = "src"

[build]
use-default-preprocessors = false

[preprocessor.index]

[preprocessor.links]

[preprocessor.alerts]

[preprocessor.image-viewer]

[preprocessor.emojicodes]

[preprocessor.toc]
command = "mdbook-toc"
renderer = ["html"]

[preprocessor.katex]
after = ["links"]
no-css = true
include-src = true

[preprocessor.admonish]
command = "mdbook-admonish"
assets_version = "3.0.1"

[preprocessor.mermaid]
command = "mdbook-mermaid"

[preprocessor.echarts]
after = ["katex"]

[preprocessor.pikchr]

[output.html]
curly-quotes = true
mathjax-support = true
additional-css = ["katex.min.css", "./theme/mdbook-admonish.css"]
additional-js = [
    "./assets/mermaid/mermaid.min.js",
    "./assets/echarts/echarts.min.js",
    "./assets/wavedrom/wavedrom.min.js",
    "./assets/wavedrom/wavedrom.css.js",
    "./assets/bytefield/bytefield-svg.js",
]
```

## 选择性构建

通过 Cargo features 选择需要的插件，减小体积和编译时间：

```bash
# 仅构建 TOC + KaTeX + Pikchr
cargo build --release --no-default-features \
    --features "pre-toc,pre-katex,pre-pikchr"

# 完整构建（默认）
cargo build --release
```

可用 features：

| 类别 | Feature | 对应插件 |
|------|---------|---------|
| 预处理器 | `pre-alerts`, `pre-emojicodes`, `pre-toc`, `pre-echarts`, `pre-langtabs`, `pre-mermaid`, `pre-katex`, `pre-admonish`, `pre-svgbob`, `pre-pikchr`, `pre-kroki`, `pre-embedify`, `pre-image-viewer`, `pre-wavedrom` | 对应同名预处理器 |
| 渲染器 | `ren-asciidoc`, `ren-linkcheck`, `ren-office`, `ren-pdf` | 对应同名渲染器 |

## 架构

```
mdbook → external preprocessor/renderer → command 字段 → mdbook-plugins
                                                              │
                                                              ├─ src/main.rs (路由分发)
                                                              ├─ src/preprocessors/ (14 插件)
                                                              └─ src/renderers/ (6 插件，pdf 含双后端：CDP + CLI)
```

- **单二进制分发**：通过 `command = "mdbook-plugins <name>"` 统一路由到对应插件，无需符号链接
- **PDF 双后端**：`backend = "chrome"`（默认，先 CDP 后 CLI 回退）
- **内置 C 库**：`vendor/pikchr.c`（283 KB，Zero-Clause BSD）通过 `cc` crate 编译
- **前端资产**：预编译 JS（ECharts / Mermaid / WaveDrom / Bytefield 等）存放在 `test/assets/`，通过 `additional-js` 引入
- **KaTeX 渲染**：通过 Node.js 子进程调用 KaTeX（需安装 `katex` npm 包）

## 目录结构

```
mdbook-plugins/
├── Cargo.toml           # 项目配置
├── build.rs             # 编译 pikchr C 库 + 部署二进制
├── vendor/
│   └── pikchr.c         # 内置 pikchr C 源码
├── src/
│   ├── main.rs          # 入口：argv[0] 路由分发
│   ├── lib.rs           # 库入口
│   ├── utils.rs         # 通用工具函数
│   ├── preprocessors/   # 14 个预处理器
│   └── renderers/       # 6 个渲染器（pdf 含双后端：CDP + CLI）
├── test/
│   ├── book.toml        # 测试配置
│   ├── bin/             # 符号链接部署目录
│   ├── src/             # 测试 Markdown 源文件
│   ├── assets/          # 前端 JS/CSS 资产
│   ├── theme/           # 主题 CSS
│   └── verify.sh        # 验证脚本
└── docs/                # 项目文档（mdbook 格式）
```

## 依赖

- **Rust**：edition 2021，需 Rust 1.70+
- **KaTeX**：（可选）`npm install katex@0.12.0`，用于服务端公式渲染
- **Chrome/Chromium**：（可选）用于 Office 和 PDF 渲染。推荐轻量级安装方式：

  ```bash
  # 方案 A：chromium-browser（snap，一行命令，推荐）
  sudo apt install chromium-browser

  # 方案 B：Playwright Chromium（纯用户态，无需 root，适合 CI/CD）
  # npx playwright install chromium
  # CHROME="$HOME/.cache/ms-playwright/chromium-*/chrome-linux64/chrome"
  ```

- **Node.js**：（可选）用于 KaTeX 服务端渲染

## 许可

本项目包含的 `vendor/pikchr.c` 采用 [Zero-Clause BSD license](https://opensource.org/licenses/0BSD)，
由 D. Richard Hipp（SQLite 作者）开发。

其余代码采用 MIT 许可证。
