# mdbook-plugins 维护文档

## 概述

`mdbook-plugins` 是一个将 16 个独立 mdbook 插件合并为单一二进制文件的项目。
通过 `argv[0]` 符号链接路由机制，一个二进制文件即可替代所有原独立插件，
大幅减少磁盘占用并简化部署。

### 核心设计

- **单二进制分发**：所有插件逻辑编译进一个 `mdbook-plugins` 二进制文件（~6.6 MB）
- **argv[0] 路由**：通过符号链接名称（如 `mdbook-admonish → mdbook-plugins`）自动分发到对应模块
- **模块化架构**：每个插件是独立的 Rust 模块，可按 feature 开关选择性编译
- **标准协议兼容**：完全遵循 mdbook 的预处理器和渲染器协议

---

## 目录结构

```
mdbook-plugins/
├── vendor/                 # 内置的第三方 C 源码
│   └── pikchr.c            # pikchr C 库（277KB，FFI 调用）
├── Cargo.toml              # 项目配置：依赖、特性、编译优化
├── build.rs                # 构建脚本（编译 pikchr C 库）
├── assets/                 # 嵌入的静态资源
│   ├── admonish.css        # mdbook-admonish 样式
│   └── alerts/
│       ├── style.css       # mdbook-alerts 样式
│       └── alerts.tmpl     # mdbook-alerts 模板
├── src/
│   ├── main.rs             # 分发器入口（argv[0] 路由）
│   ├── lib.rs              # 库根模块，插件注册表
│   ├── utils.rs            # 共享工具：标准预处理/渲染协议实现
│   ├── preprocessors/      # 所有预处理器模块
│   │   ├── mod.rs          # 模块索引
│   │   ├── admonish.rs     # Admonition 提示框
│   │   ├── alerts.rs       # GitHub 风格 Alert
│   │   ├── echarts.rs      # ECharts 图表
│   │   ├── embedify.rs     # 嵌入内容（YouTube/CodePen/Giscus）
│   │   ├── emojicodes.rs   # Emoji shortcode 替换
│   │   ├── katex.rs        # LaTeX 数学公式
│   │   ├── kroki.rs        # Kroki 在线图渲染
│   │   ├── langtabs.rs     # 语言标签页
│   │   ├── mermaid.rs      # Mermaid 流程图
│   │   ├── pikchr.rs       # Pikchr 图（FFI 调用 C 库）
│   │   ├── svgbob.rs       # ASCII art → SVG
│   │   ├── toc.rs          # 目录生成
│   │   └── wavedrom.rs     # WaveDrom 时序图
│   └── renderers/          # 所有渲染器模块
│       ├── mod.rs          # 模块索引
│       ├── asciidoc.rs     # AsciiDoc 输出
│       ├── linkcheck.rs    # 链接检查
│       └── office.rs       # Office 文档（DOCX/XLSX/PPTX）
```

---

## 依赖关系

### 关键 Crate

| Crate | 用途 | 涉及的插件 |
|-------|------|-----------|
| `mdbook 0.4` | mdbook 核心类型和协议 | 所有插件 |
| `mdbook-preprocessor 0.5` | 预处理器抽象 | mermaid, toc, katex 等 |
| `mdbook-renderer 0.5` | 渲染器抽象 | asciidoc |
| `pulldown-cmark 0.13` | Markdown 解析 | admonish, svgbob, pikchr, wavedrom 等 |
| `regex 1` | 正则匹配 | alerts, echarts, katex 等 |
| `tokio 1` + `reqwest 0.12` | 异步 HTTP | kroki, linkcheck |
| `svgbob 0.7` | ASCII art 转 SVG | svgbob |
| `libc` | C FFI | pikchr |
| `emojis 0.6` | Emoji 数据库 | emojicodes |

### pikchr C 依赖

pikchr 使用内置的 C 源文件（`vendor/pikchr.c`），在 `build.rs` 中通过 `cc` crate 自动编译。
需要系统已安装 C 编译器（gcc 或 clang）。

```
vendor/
└── pikchr.c          # 内置的 pikchr C 源文件（277KB，8247 行）
```

如果系统中没有 C 编译器，可以在构建时禁用 pikchr：

```bash
cargo build --release --no-default-features \
    --features "pre-alerts pre-emojicodes pre-toc pre-echarts ...(其余特性)"
```

---

## 插件协议说明

### 预处理器协议

每个预处理器模块必须导出：

```rust
/// 标准入口函数，无参数，从 stdin 读取，输出到 stdout
pub fn run() -> anyhow::Result<()>;

/// 插件主体（可选导出，用于 supports 检查）
pub struct XxxPreprocessor;

impl mdbook::preprocess::Preprocessor for XxxPreprocessor {
    fn name(&self) -> &str;
    fn supports_renderer(&self, renderer: &str) -> bool;
    fn run(&self, ctx: &PreprocessorContext, book: Book) -> Result<Book, Error>;
}
```

mdbook 调用预处理器的方式：

```
$ mdbook-admonish                     # 预处理模式：从 stdin 读取 JSON，输出到 stdout
$ mdbook-admonish supports html       # 检查是否支持 html 渲染器（退出码 0/1）
```

### 渲染器协议

每个渲染器模块必须导出：

```rust
pub fn run() -> anyhow::Result<()>;
```

渲染器实现 `mdbook::renderer::Renderer` trait，从 stdin 读取 `RenderContext`。

---

## 构建指南

### 开发构建

```bash
cd /home/kuanghl/workspace/rpp/repo/mdbook-plugins
cargo build
```

生成 `target/debug/mdbook-plugins`（Debug 模式，~133 MB）

### 发布构建

```bash
cargo build --release
```

生成 `target/release/mdbook-plugins`（Release 模式，~6.6 MB，启用了 LTO + strip）

### 选择性编译（特性开关）

```bash
# 只编译 alerts 和 toc 预处理器
cargo build --release --no-default-features --features "pre-alerts pre-toc"

# 只编译 linkcheck 渲染器
cargo build --release --no-default-features --features "ren-linkcheck"

# 编译除 pikchr 外的所有插件
cargo build --release --no-default-features \
    --features "pre-alerts pre-emojicodes pre-toc pre-echarts pre-langtabs \
               pre-mermaid pre-katex pre-admonish pre-svgbob pre-kroki \
               pre-embedify pre-wavedrom ren-asciidoc ren-linkcheck"
```

可用特性：

| 特性 | 对应插件 | 类型 |
|------|---------|------|
| `pre-alerts` | mdbook-alerts | 预处理器 |
| `pre-emojicodes` | mdbook-emojicodes | 预处理器 |
| `pre-toc` | mdbook-toc | 预处理器 |
| `pre-echarts` | mdbook-echarts | 预处理器 |
| `pre-langtabs` | mdbook-langtabs | 预处理器 |
| `pre-mermaid` | mdbook-mermaid | 预处理器 |
| `pre-katex` | mdbook-katex | 预处理器 |
| `pre-admonish` | mdbook-admonish | 预处理器 |
| `pre-svgbob` | mdbook-svgbob | 预处理器 |
| `pre-pikchr` | mdbook-pikchr | 预处理器（需 C 编译器） |
| `pre-kroki` | mdbook-kroki-preprocessor | 预处理器 |
| `pre-embedify` | mdbook-embedify | 预处理器 |
| `pre-wavedrom` | mdbook-wavedrom-rs | 预处理器 |
| `ren-asciidoc` | mdbook-asciidoc | 渲染器 |
| `ren-linkcheck` | mdbook-linkcheck | 渲染器 |
| `ren-office` | mdbook-office | 渲染器（office_oxide）|

---

## 部署到 mdbook-demo

### 快速部署

```bash
# 1. 构建
cd /home/kuanghl/workspace/rpp/repo/mdbook-plugins
cargo build --release

# 2. 复制二进制
cp target/release/mdbook-plugins /home/kuanghl/workspace/rpp/repo/mdbook-demo/bin/

# 3. 备份并创建符号链接
cd /home/kuanghl/workspace/rpp/repo/mdbook-demo/bin
BACKUP_DIR="backup_$(date +%Y%m%d_%H%M%S)"
mkdir -p "$BACKUP_DIR"

for name in \
    mdbook-admonish mdbook-alerts mdbook-echarts mdbook-emojicodes \
    mdbook-embedify mdbook-katex mdbook-kroki-preprocessor \
    mdbook-langtabs mdbook-mermaid mdbook-pikchr mdbook-svgbob \
    mdbook-toc mdbook-wavedrom-rs \
    mdbook-asciidoc mdbook-linkcheck mdbook-office; do
    if [ -f "$name" ] && [ ! -L "$name" ]; then
        mv "$name" "$BACKUP_DIR/"
    fi
    ln -sf mdbook-plugins "$name"
done
```

### 验证部署

```bash
cd /home/kuanghl/workspace/rpp/repo/mdbook-demo
PATH="./bin:$PATH" mdbook-admonish supports html
echo "退出码: $?"   # 应该为 0

PATH="./bin:$PATH" mdbook-admonish supports not-supported
echo "退出码: $?"   # 应该为 1

# 完整构建测试（跳过 PDF 后端以加速）
# 临时注释掉 book.toml 中的 [output.pdf]
PATH="./bin:$PATH" mdbook build
```

### 验证构建输出

```bash
ls /home/kuanghl/workspace/rpp/repo/mdbook-demo/books/index.html
```

---

## 添加新插件

### 步骤

1. **创建模块文件**：在 `src/preprocessors/` 或 `src/renderers/` 下创建 `.rs` 文件

2. **注册模块**：
   - 在 `preprocessors/mod.rs` 或 `renderers/mod.rs` 中添加 `pub mod your_plugin;`
   - 在 `lib.rs` 的 `PLUGIN_NAMES` 数组中添加插件名

3. **实现标准接口**：
   - 预处理器：实现 `Preprocessor` trait + `pub fn run()`
   - 渲染器：实现 `Renderer` trait + `pub fn run()`

4. **注册路由**：在 `main.rs` 的 `run_plugin()` 函数中添加 match 分支

5. **添加特性**：在 `Cargo.toml` 的 `[features]` 中添加对应的特性开关，并加入 `all` 列表

6. **添加依赖**：如果新插件需要额外的 crate，添加到 `[dependencies]` 中

### 模板

预处理器模板：

```rust
//! src/preprocessors/your_plugin.rs

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{Preprocessor, PreprocessorContext};

pub struct YourPreprocessor;

impl Preprocessor for YourPreprocessor {
    fn name(&self) -> &str {
        "mdbook-your-plugin"
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer != "not-supported"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        book.for_each_mut(|item: &mut BookItem| {
            if let BookItem::Chapter(ref mut chapter) = item {
                // 处理 chapter.content
            }
        });
        Ok(book)
    }
}

pub fn run() -> anyhow::Result<()> {
    let pre = YourPreprocessor;
    crate::utils::run_preprocessor(&pre)
}
```

渲染器模板：

```rust
//! src/renderers/your_renderer.rs

use mdbook::errors::Error;
use mdbook::renderer::{RenderContext, Renderer};

pub struct YourRenderer;

impl Renderer for YourRenderer {
    fn name(&self) -> &str {
        "your-renderer"
    }

    fn render(&self, ctx: &RenderContext) -> Result<(), Error> {
        let book = &ctx.book;
        let destination = &ctx.destination;
        // 渲染逻辑
        Ok(())
    }
}

pub fn run() -> anyhow::Result<()> {
    let renderer = YourRenderer;
    crate::utils::run_renderer(&renderer)
}
```

---

## 常见问题

### Q: pikchr 编译失败怎么办？

A: pikchr 需要 C 编译器（gcc/clang）和内置的 `vendor/pikchr.c` 源文件。如果缺少 C 编译器：

```bash
# 安装编译器（Ubuntu/Debian）
sudo apt install build-essential

# 或禁用 pikchr 特性重新构建
cargo build --release --no-default-features --features "...(除 pikchr 外的所有特性)..."
```

### Q: 构建体积过大怎么办？

A: Release 模式默认启用 LTO 和 strip，体积已优化。如果仍有体积顾虑：

- 使用 `--no-default-features` 只编译需要的插件
- 或在 `[profile.release]` 中调整优化级别

### Q: 符号链接不生效？

A: 确保符号链接指向 `mdbook-plugins`，且 `mdbook-plugins` 在同一目录：

```bash
# 检查符号链接
ls -la /home/kuanghl/workspace/rpp/repo/mdbook-demo/bin/mdbook-admonish
# 输出: mdbook-admonish -> mdbook-plugins

# 确保目标存在
ls -l /home/kuanghl/workspace/rpp/repo/mdbook-demo/bin/mdbook-plugins
```

### Q: mdbook 找不到插件？

A: 确保 `bin/` 目录在 PATH 中：

```bash
cd /home/kuanghl/workspace/rpp/repo/mdbook-demo
PATH="./bin:$PATH" mdbook build
```

或者在 `book.toml` 中为每个预处理器显式指定 `command`：

```toml
[preprocessor.admonish]
command = "mdbook-admonish"
```

---

## 版本兼容性

| 组件 | 要求 |
|------|------|
| mdbook | >= 0.4.36（测试版本），兼容 0.4.x 系列 |
| Rust 编译器 | >= 1.76（edition 2021） |
| C 编译器（仅 pikchr） | gcc 或 clang |

项目中的 `MDBOOK_VERSION` 警告是正常现象，不影响功能。它仅提示编译时和运行时的 mdbook 版本号不同。

---

## 备份与回滚

所有原始独立二进制在部署时自动备份到 `bin/backup_<timestamp>/`。

### 回滚步骤

```bash
cd /home/kuanghl/workspace/rpp/repo/mdbook-demo/bin

# 找到最近的备份
ls -d backup_*

# 恢复所有原始二进制
BACKUP="backup_20260717_112437"  # 替换为实际备份目录
for file in "$BACKUP"/mdbook-*; do
    name=$(basename "$file")
    rm -f "$name"          # 删除符号链接
    cp "$file" "$name"     # 恢复原始二进制
done
```

---

## 各插件实现要点

| 插件 | 实现方式 | 特殊依赖 |
|------|---------|---------|
| admonish | pulldown-cmark 解析 admonish 代码块 | CSS 资源（`assets/admonish.css`）|
| alerts | 正则匹配 `> [!KIND]` 语法 | CSS 和模板资源 |
| echarts | 正则匹配 ` ```echarts ` 和 `{% echarts %}` | uuid 生成唯一 ID |
| emojicodes | 正则匹配 `:shortcode:` + `emojis` crate | — |
| embedify | 正则匹配 `{% tag %}` 语法 | — |
| katex | 字符流逐个解析 `$..$` 和 `$$..$$` | 不支持 lookaround 正则 |
| kroki | base64 编码 + HTTP POST 到 kroki.io | tokio, reqwest, flate2, base64 |
| langtabs | 正则匹配 `<!-- langtabs -->` 标记 | — |
| mermaid | 正则匹配 ` ```mermaid ` 代码块 | — |
| pikchr | pulldown-cmark 解析 + FFI 调用 C 库 | C 编译器, libc |
| svgbob | pulldown-cmark 解析 + svgbob crate | svgbob, svg |
| toc | pulldown-cmark 解析 heading | — |
| wavedrom | pulldown-cmark 解析 wavedrom 代码块 | — |
| asciidoc | pulldown-cmark 解析 → AsciiDoc 语法映射 | — |
| linkcheck | tokio + reqwest 异步 HTTP 检查 | tokio, reqwest, futures |
| office | office_oxide 库生成 DOCX/XLSX/PPTX | office_oxide |

---

*文档版本: 0.1.0 | 最后更新: 2026-07-17*
