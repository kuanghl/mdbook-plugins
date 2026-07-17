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
| **mdbook-pdf** | 通过 Chrome headless 生成 PDF |

## 快速开始

### 构建

```bash
git clone <repo-url>
cd mdbook-plugins

# 安装 KaTeX 依赖（用于 mdbook-katex 服务端预渲染）
npm install katex@0.12.0

# 构建（Release 模式）
cargo build --release
```

构建完成后，二进制自动部署到 `test/bin/mdbook-plugins`（通过 `build.rs`）。

### 部署

为每个插件创建符号链接指向 `mdbook-plugins`：

```bash
cd test/bin
for name in mdbook-admonish mdbook-alerts mdbook-echarts mdbook-emojicodes \
    mdbook-embedify mdbook-katex mdbook-kroki-preprocessor mdbook-langtabs \
    mdbook-mermaid mdbook-pikchr mdbook-svgbob mdbook-toc mdbook-wavedrom-rs \
    mdbook-asciidoc mdbook-linkcheck mdbook-office mdbook-pdf; do
    ln -sf mdbook-plugins "$name"
done
```

### 运行测试

```bash
cd test
./verify.sh --full
```

测试包含：
- 17 个插件的 `supports` 协议测试
- 路由正确性测试
- 完整 mdbook 构建测试

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
mdbook → external preprocessor/renderer → argv[0] 符号链接 → mdbook-plugins
                                                              │
                                                              ├─ src/main.rs (路由分发)
                                                              ├─ src/preprocessors/ (13 插件)
                                                              └─ src/renderers/ (4 插件)
```

- **单二进制分发**：通过 `argv[0]` 名称路由到对应插件
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
│   └── renderers/       # 4 个渲染器
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
- **Chrome/Chromium**：（可选）用于 PDF 和 Office 渲染
- **Node.js**：（可选）用于 KaTeX 服务端渲染

## 许可

本项目包含的 `vendor/pikchr.c` 采用 [Zero-Clause BSD license](https://opensource.org/licenses/0BSD)，
由 D. Richard Hipp（SQLite 作者）开发。

其余代码采用 MIT 许可证。
