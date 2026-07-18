# 架构设计

## 整体架构

mdbook-plugins 采用**单二进制 + command 路由**架构。
所有插件代码编译进一个可执行文件，运行时通过 `command = "mdbook-plugins <name>"` 参数分发到对应的处理逻辑。无需符号链接。

```mermaid
graph TB
    subgraph "mdbook-plugins 二进制"
        DISPATCH["分发器 main.rs"]
        SUB["supports 子命令处理"]
        PREPROC["预处理器模块组"]
        RENDER["渲染器模块组"]
    end

    subgraph "book.toml command 路由"
        CMD_ADMON["[preprocessor.admonish]\ncommand = \"mdbook-plugins admonish\""]
        CMD_TOC["[preprocessor.toc]\ncommand = \"mdbook-plugins toc\""]
        CMD_PDF["[output.pdf]\ncommand = \"mdbook-plugins pdf\""]
        CMD_OTHER["... 其他 17 个插件"]
    end

    subgraph "mdbook 调用方式"
        MDBOOK["mdbook build"]
        PRE_PROTO["stdin: PreprocessorContext + Book<br/>stdout: Book JSON"]
        REN_PROTO["stdin: RenderContext"]
    end

    MDBOOK --> CMD_ADMON
    MDBOOK --> CMD_TOC
    MDBOOK --> CMD_PDF
    MDBOOK --> CMD_OTHER

    CMD_ADMON --> DISPATCH
    CMD_TOC --> DISPATCH
    CMD_PDF --> DISPATCH
    CMD_OTHER --> DISPATCH

    DISPATCH -->|"supports 参数"| SUB
    DISPATCH -->|"argv[0] = mdbook-admonish"| PREPROC
    DISPATCH -->|"argv[0] = mdbook-asciidoc"| RENDER

    PREPROC -->|读 stdin, 写 stdout| PRE_PROTO
    RENDER -->|读 stdin| REN_PROTO
```

## 分发机制

### command 路由

当 mdbook 调用插件时，通过 `command` 字段传入 `mdbook-plugins <name>` 参数。
Rust 程序读取 `argv[1]` 作为插件名进行路由。

```rust
// main.rs 核心逻辑（简化）
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (plugin_name, plugin_args) = resolve_plugin(&args);
    run_plugin(&plugin_name, &plugin_args);
}

// resolve_plugin 逻辑：
// - argv[1] 是已知短名称（如 "katex"）→ 组装为 "mdbook-<name>"
// - argv[1] 是 "mdbook-xxx" 格式 → 直接使用
// - 否则从 argv[0]（符号链接名）推断（兼容旧式部署）
```

### 参数传递规则

| 场景 | argv[0] | argv[1] | 行为 |
|------|---------|---------|------|
| 正常预处理 | `mdbook-plugins` | `katex` | 路由到 katex 模块 |
| supports 检查 | `mdbook-plugins` | `katex` `supports` `html` | 路由到 katex 的 supports 处理 |
| 渲染器调用 | `mdbook-plugins` | `pdf` | 路由到 pdf 模块 |
| 符号链接兼容 | `mdbook-toc` | — | 用 argv[0] 推断插件名 |

### 预处理器分类

```mermaid
graph LR
    subgraph 预处理器
        A["admonish<br/>提示框"]
        B["alerts<br/>Alert 块"]
        C["echarts<br/>图表"]
        D["emojicodes<br/>Emoji 替换"]
        E["embedify<br/>嵌入内容"]
        F["katex<br/>数学公式"]
        G["kroki<br/>Kroki 图"]
        H["langtabs<br/>语言标签"]
        I["mermaid<br/>流程图"]
        J["pikchr<br/>Pikchr 图"]
        K["svgbob<br/>ASCII 转 SVG"]
        L["toc<br/>目录生成"]
        M["wavedrom<br/>时序图"]
    end

    subgraph 渲染器
        N["asciidoc<br/>AsciiDoc 输出"]
        O["linkcheck<br/>链接检查"]
        P["office<br/>Office 文档"]
        Q["pdf<br/>PDF 双后端<br/>CDP / CLI"]
    end
```

## 模块结构

```
src/
├── main.rs                  # 分发器入口
├── lib.rs                   # 库根 + 插件注册表
├── utils.rs                 # 共享工具函数
├── preprocessors/
│   ├── mod.rs               # 13 个预处理器模块索引
│   ├── admonish.rs          # Admonition
│   ├── alerts.rs            # GitHub Alerts
│   ├── echarts.rs           # ECharts
│   ├── embedify.rs          # 嵌入内容
│   ├── emojicodes.rs        # Emoji
│   ├── katex.rs             # KaTeX
│   ├── kroki.rs             # Kroki
│   ├── langtabs.rs          # 语言标签
│   ├── mermaid.rs           # Mermaid
│   ├── pikchr.rs            # Pikchr（C FFI）
│   ├── svgbob.rs            # Svgbob
│   ├── toc.rs               # 目录
│   └── wavedrom.rs          # WaveDrom
└── renderers/
    ├── mod.rs               # 4 个渲染器模块索引
    ├── asciidoc.rs           # AsciiDoc
    ├── linkcheck.rs          # 链接检查
    ├── office.rs             # Office 文档
    └── pdf.rs + pdf_chrome_cdp.rs + pdf_html_preprocess.rs  # PDF 三模块（CDP 双模式 + HTML 预处理）
```

### 模块接口约定

每个预处理器模块导出：

```rust
/// 标准入口函数（无参数，从 stdin 读，向 stdout 写）
pub fn run() -> anyhow::Result<()>;

/// 插件主体（实现 Preprocessor trait）
pub struct XxxPreprocessor;
impl mdbook::preprocess::Preprocessor for XxxPreprocessor { ... }
```

每个渲染器模块导出：

```rust
/// 标准入口函数
pub fn run() -> anyhow::Result<()>;
```

## 预处理协议

mdbook 预处理器通过 stdin/stdout 与 mdbook 通信：

```mermaid
sequenceDiagram
    participant M as mdbook
    participant P as mdbook-plugins (preprocessor)

    Note over M,P: 预处理阶段
    M->>P: stdin: 写入 PreprocessorContext + Book (JSON)
    activate P
    Note over P: 解析输入<br/>处理章节内容<br/>替换/转换 Markdown
    P-->>M: stdout: 输出处理后的 Book (JSON)
    deactivate P

    Note over M,P: supports 检查阶段
    M->>P: mdbook-admonish supports html
    activate P
    P-->>M: exit(0) 支持 / exit(1) 不支持
    deactivate P
```

### Preprocessor trait

```rust
pub trait Preprocessor {
    /// 插件名称
    fn name(&self) -> &str;

    /// 检查是否支持某渲染器
    fn supports_renderer(&self, renderer: &str) -> bool;

    /// 核心处理逻辑
    fn run(&self, ctx: &PreprocessorContext, book: Book) -> Result<Book, Error>;
}
```

## 渲染协议

渲染器同样通过 stdin 接收 `RenderContext`，处理结果写入文件系统：

```rust
pub trait Renderer {
    fn name(&self) -> &str;
    fn render(&self, ctx: &RenderContext) -> Result<(), Error>;
}
```

### 共享工具函数

`src/utils.rs` 提供了标准入口封装，减少模板代码：

```rust
/// 预处理器标准入口
pub fn run_preprocessor<P: Preprocessor>(pre: &P) -> anyhow::Result<()> {
    let (ctx, book) = CmdPreprocessor::parse_input(std::io::stdin())?;
    // 版本兼容性检查...
    let processed = pre.run(&ctx, book)?;
    serde_json::to_writer(std::io::stdout(), &processed)?;
    Ok(())
}

/// 渲染器标准入口
pub fn run_renderer<R: Renderer>(renderer: &R) -> anyhow::Result<()> {
    let ctx = RenderContext::from_json(std::io::stdin())?;
    renderer.render(&ctx)?;
    Ok(())
}
```
