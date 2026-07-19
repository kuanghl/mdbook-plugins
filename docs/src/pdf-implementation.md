# mdbook-pdf 纯 Rust 实现技术方案

## 概述

`mdbook-pdf` 通过 **Chrome DevTools Protocol (CDP)** 生成高质量 PDF，解决了页眉/页脚重叠、内容不当分页等核心痛点。渲染器由三个模块协同工作。

[基于 Chrome DevTools Protocol Page.printToPDF 的完整参数](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-printToPDF)

> **项目目标**：使用纯 Rust 实现与 [mdbook-pdf](https://github.com/HollowMan6/mdbook-pdf.git) 相同功能的 mdBook PDF 后端插件，基于 Headless Chrome + Chrome DevTools Protocol (CDP) 生成高质量 PDF。[[10]]

---

## 目录

1. [系统架构总览](#1-系统架构总览)
2. [功能清单](#2-功能清单)
3. [CDP Page.printToPDF 参数规范](#3-cdp-pageprinttopdf-参数规范)
4. [模块设计与实现方案](#4-模块设计与实现方案)
5. [数据流图](#5-数据流图)
6. [核心依赖库](#6-核心依赖库)
7. [配置系统设计](#7-配置系统设计)
8. [关键算法与实现细节](#8-关键算法与实现细节)
9. [错误处理与重试机制](#9-错误处理与重试机制)
10. [测试策略](#10-测试策略)
11. [项目目录结构](#11-项目目录结构)

---

## 1. 系统架构总览

### 1.1 高层架构图

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          mdbook build                                    │
│                              │                                           │
│                              ▼                                           │
│                    ┌──────────────────┐                                  │
│                    │  RenderContext   │  (stdin JSON)                    │
│                    │  (mdbook 传入)   │                                  │
│                    └────────┬─────────┘                                  │
│                             │                                            │
│                             ▼                                            │
│              ┌──────────────────────────────┐                           │
│              │      PdfRenderer (入口)       │                           │
│              │      pdf.rs::run_pdf()       │                           │
│              └──────────────┬───────────────┘                           │
│                             │                                            │
│            ┌────────────────┼────────────────┐                          │
│            │                │                │                           │
│            ▼                ▼                ▼                           │
│   ┌──────────────┐  ┌────────────┐  ┌──────────────────┐              │
│   │ 配置解析     │  │ 章节提取   │  │ 元数据提取       │              │
│   │ (book.toml)  │  │ (paths)    │  │ (title/authors)  │              │
│   └──────┬───────┘  └─────┬──────┘  └────────┬─────────┘              │
│          │                 │                   │                         │
│          └─────────────────┼───────────────────┘                         │
│                            │                                             │
│                            ▼                                             │
│              ┌──────────────────────────────┐                           │
│              │     后端调度 (Dispatcher)     │                           │
│              └──────────────┬───────────────┘                           │
│                             │                                            │
│              ┌──────────────┴──────────────┐                            │
│              │                             │                             │
│              ▼                             ▼                             │
│   ┌─────────────────────┐    ┌─────────────────────┐                   │
│   │  Chrome CDP 后端    │    │  Chrome CLI 后端    │                   │
│   │  (主路径)           │    │  (降级回退)         │                   │
│   │                     │    │                     │                   │
│   │ ┌───────────────┐   │    │ ┌───────────────┐   │                   │
│   │ │ HTML 预处理   │   │    │ │ @page CSS     │   │                   │
│   │ │ (preprocess)  │   │    │ │ 注入          │   │                   │
│   │ └───────┬───────┘   │    │ └───────┬───────┘   │                   │
│   │         │            │    │         │            │                   │
│   │         ▼            │    │         ▼            │                   │
│   │ ┌───────────────┐   │    │ ┌───────────────┐   │                   │
│   │ │ CDP 通信      │   │    │ │ --headless    │   │                   │
│   │ │ printToPDF    │   │    │ │ --print-to-pdf│   │                   │
│   │ └───────┬───────┘   │    │ └───────┬───────┘   │                   │
│   │         │            │    │         │            │                   │
│   └─────────┼────────────┘    └─────────┼────────────┘                  │
│             │                           │                                │
│             └─────────────┬─────────────┘                                │
│                           │                                              │
│                           ▼                                              │
│              ┌──────────────────────────────┐                           │
│              │     PDF 后处理               │                           │
│              │     (pdf_outline.rs)         │                           │
│              │                              │                           │
│              │  ┌────────────────────────┐  │                           │
│              │  │ 书签/大纲生成          │  │                           │
│              │  │ (extract + add)        │  │                           │
│              │  └────────────────────────┘  │                           │
│              │  ┌────────────────────────┐  │                           │
│              │  │ PDF 元数据写入         │  │                           │
│              │  │ (title/author/lang)    │  │                           │
│              │  └────────────────────────┘  │                           │
│              └──────────────┬───────────────┘                           │
│                             │                                            │
│                             ▼                                            │
│                    ┌──────────────────┐                                  │
│                    │   output.pdf     │                                  │
│                    └──────────────────┘                                  │
└─────────────────────────────────────────────────────────────────────────┘
```

### 1.2 模块依赖关系图

```
┌─────────────────────────────────────────────────────────────────┐
│                        main.rs / lib.rs                          │
│                     (入口 + 模块注册)                             │
└──────────────────────────────┬──────────────────────────────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │                    │                    │
          ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│    utils.rs     │  │    pdf.rs       │  │   (其他插件)    │
│  (通用工具)     │  │  (PDF渲染器)    │  │                 │
└─────────────────┘  └────────┬────────┘  └─────────────────┘
                              │
              ┌───────────────┼───────────────┐
              │               │               │
              ▼               ▼               ▼
   ┌────────────────┐ ┌──────────────┐ ┌────────────────────┐
   │pdf_chrome_cdp  │ │pdf_html_     │ │  pdf_outline.rs    │
   │.rs             │ │preprocess.rs │ │  (PDF后处理)       │
   │(CDP后端)       │ │(HTML预处理)  │ │  (书签+元数据)     │
   └────────────────┘ └──────────────┘ └────────────────────┘
```

---

## 2. 功能清单

### 2.1 核心功能矩阵

| # | 功能模块 | 功能项 | 优先级 | 说明 |
|---|---------|--------|--------|------|
| 1 | **PDF 生成** | Chrome CDP 后端 | P0 | 通过 `Page.printToPDF` 协议生成 PDF |
| 2 | **PDF 生成** | Chrome CLI 后端 | P0 | `--headless --print-to-pdf` 降级方案 |
| 3 | **PDF 生成** | 后端自动回退 | P0 | CDP 失败时自动切换 CLI |
| 4 | **PDF 生成** | 重试机制 | P1 | 可配置重试次数 (`trying-times`) |
| 5 | **页面布局** | 纸张尺寸 | P0 | 自定义宽高（英寸） |
| 6 | **页面布局** | 页面方向 | P0 | 纵向/横向 |
| 7 | **页面布局** | 页边距 | P0 | 四边独立设置 |
| 8 | **页面布局** | 缩放比例 | P1 | 全局缩放因子 |
| 9 | **页面布局** | CSS @page 优先 | P2 | 以 CSS 定义的页面尺寸为准 |
| 10 | **页眉/页脚** | CDP 原生模式 | P0 | `headerTemplate`/`footerTemplate` |
| 11 | **页眉/页脚** | CSS 注入模式 | P1 | `position:fixed` + `@page` 边距补偿 |
| 12 | **页眉/页脚** | 模式组合 | P1 | 4 种组合模式（原生/CSS/双/无） |
| 13 | **页眉/页脚** | 模板变量 | P1 | `date`/`title`/`url`/`pageNumber`/`totalPages` |
| 14 | **内容控制** | 背景打印 | P0 | 打印背景色/背景图 |
| 15 | **内容控制** | 页码范围 | P1 | 如 `"1-5,8,11-13"` |
| 16 | **内容控制** | 分页保护 | P1 | 代码块/表格/图片不断页 |
| 17 | **书签/大纲** | 标题提取 | P0 | 从 HTML 解析 h1-h6 结构 |
| 18 | **书签/大纲** | 命名目标解析 | P0 | 解析 PDF 中的 Named Destinations |
| 19 | **书签/大纲** | 书签树构建 | P0 | 层级书签（父子关系） |
| 20 | **书签/大纲** | Chrome 原生大纲 | P2 | `generateDocumentOutline` 参数 |
| 21 | **元数据** | PDF 元信息 | P1 | Title/Author/Subject/Language |
| 22 | **HTML 预处理** | ToC 锚点注入 | P0 | 为章节创建 PDF 命名目标 |
| 23 | **HTML 预处理** | JS 注入 | P1 | 展开 `<details>`、MathJax 挂钩 |
| 24 | **HTML 预处理** | 链接修正 | P1 | 相对路径 → 绝对 URL |
| 25 | **HTML 预处理** | 字体 CSS 注入 | P1 | CJK 字体回退，避免方框乱码 |
| 26 | **HTML 预处理** | 打印 CSS 注入 | P1 | `@media print` 分页控制 |
| 27 | **浏览器管理** | 自动探测 | P0 | 多平台 Chrome/Chromium/Edge 检测 |
| 28 | **浏览器管理** | 环境变量 | P1 | `CHROME` 环境变量 |
| 29 | **浏览器管理** | 配置路径 | P1 | `browser-binary-path` |
| 30 | **浏览器管理** | 自动下载 | P2 | `fetch` feature 自动下载 Chromium |

### 2.2 页眉/页脚四种组合模式

```
┌─────────────────────────────────────────────────────────────────────┐
│                    页眉/页脚模式组合矩阵                              │
├─────────────────────────┬───────────────────┬───────────────────────┤
│                         │ css-header-footer │ css-header-footer     │
│                         │    = true         │    = false            │
├─────────────────────────┼───────────────────┼───────────────────────┤
│ use-native-header-footer│                   │                       │
│       = false           │  模式 1 (默认)    │  模式 2               │
│                         │  仅 CSS 注入      │  无页眉/页脚          │
│                         │  position:fixed   │                       │
├─────────────────────────┼───────────────────┼───────────────────────┤
│ use-native-header-footer│                   │                       │
│       = true            │  模式 3           │  模式 4 (推荐)        │
│                         │  CDP原生+CSS注入  │  仅 CDP 原生          │
│                         │  (双页眉/页脚)    │  headerTemplate       │
└─────────────────────────┴───────────────────┴───────────────────────┘

优先级覆盖：no-header = true → 强制禁用所有页眉/页脚
```

---

## 3. CDP Page.printToPDF 参数规范

> 来源：[Chrome DevTools Protocol - Page.printToPDF](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-printToPDF) [[2]]

### 3.1 请求参数表

| 参数名 | 类型 | 默认值 | 必填 | 说明 | 对应 book.toml 配置 |
|--------|------|--------|------|------|---------------------|
| `landscape` | `boolean` | `false` | 否 | 纸张方向。`false`=纵向，`true`=横向 | `landscape` |
| `displayHeaderFooter` | `boolean` | `false` | 否 | 是否显示页眉和页脚 | `display-header-footer` |
| `printBackground` | `boolean` | `false` | 否 | 是否打印背景图形（背景色/背景图） | `print-background` |
| `scale` | `number` | `1` | 否 | 网页渲染缩放比例 | `scale` |
| `paperWidth` | `number` | `8.5` | 否 | 纸张宽度（英寸） | `paper-width` |
| `paperHeight` | `number` | `11` | 否 | 纸张高度（英寸） | `paper-height` |
| `marginTop` | `number` | `~0.4` (1cm) | 否 | 上边距（英寸） | `margin-top` |
| `marginBottom` | `number` | `~0.4` (1cm) | 否 | 下边距（英寸） | `margin-bottom` |
| `marginLeft` | `number` | `~0.4` (1cm) | 否 | 左边距（英寸） | `margin-left` |
| `marginRight` | `number` | `~0.4` (1cm) | 否 | 右边距（英寸） | `margin-right` |
| `pageRanges` | `string` | `""` (全部) | 否 | 打印页码范围，如 `"1-5, 8, 11-13"`。页码从 1 开始，按文档顺序打印，超出范围的页码被静默忽略。起始大于结束时报错 | `page-range` |
| `headerTemplate` | `string` | `""` | 否 | 页眉 HTML 模板。支持 class：`date`、`title`、`url`、`pageNumber`、`totalPages`。例如 `<span class=title></span>` | `header-template` |
| `footerTemplate` | `string` | `""` | 否 | 页脚 HTML 模板。格式同 `headerTemplate` | `footer-template` |
| `preferCSSPageSize` | `boolean` | `false` | 否 | 是否优先使用 CSS 定义的页面尺寸。`false` 时内容缩放适配纸张 | `prefer-css-page-size` |
| `transferMode` | `string` | `"ReturnAsBase64"` | 否 | 返回模式。可选值：`ReturnAsBase64`、`ReturnAsStream`（实验性） | — |
| `generateTaggedPDF` | `boolean` | 嵌入器决定 | 否 | 是否生成带标签的 PDF（无障碍支持）（实验性） | `generate-tagged-pdf` |
| `generateDocumentOutline` | `boolean` | — | 否 | 是否在 PDF 中嵌入文档大纲/书签（实验性） | `generate-document-outline` |

### 3.2 返回值

| 字段名 | 类型 | 说明 |
|--------|------|------|
| `data` | `string` | Base64 编码的 PDF 数据。当 `transferMode=ReturnAsStream` 时为空 |
| `stream` | `IO.StreamHandle` | PDF 数据流句柄（实验性，仅 `ReturnAsStream` 模式） |

### 3.3 headerTemplate / footerTemplate 支持的 CSS 类

| CSS 类名 | 注入内容 | 示例 |
|----------|---------|------|
| `date` | 格式化的打印日期 | `<span class="date"></span>` |
| `title` | 文档标题 | `<span class="title"></span>` |
| `url` | 文档 URL | `<span class="url"></span>` |
| `pageNumber` | 当前页码 | `<span class="pageNumber"></span>` |
| `totalPages` | 总页数 | `<span class="totalPages"></span>` |

### 3.4 参数映射关系图

```
book.toml [output.pdf]              CDP Page.printToPDF
─────────────────────────           ─────────────────────
landscape = false          ──────►  landscape: false
display-header-footer = t  ──────►  displayHeaderFooter: true
print-background = true    ──────►  printBackground: true
scale = 1.0               ──────►  scale: 1.0
paper-width = 8.5         ──────►  paperWidth: 8.5
paper-height = 11.0       ──────►  paperHeight: 11.0
margin-top = 1.0          ──────►  marginTop: 1.0
margin-bottom = 1.0       ──────►  marginBottom: 1.0
margin-left = 1.0         ──────►  marginLeft: 1.0
margin-right = 1.0        ──────►  marginRight: 1.0
page-range = "1-5"        ──────►  pageRanges: "1-5"
header-template = "..."   ──────►  headerTemplate: "..."
footer-template = "..."   ──────►  footerTemplate: "..."
prefer-css-page-size = f  ──────►  preferCSSPageSize: false
generate-tagged-pdf = t   ──────►  generateTaggedPDF: true
generate-document-outline ──────►  (后处理模块负责，不传 CDP)
```

---

## 4. 模块设计与实现方案

### 4.1 模块一：入口与渲染器 (`pdf.rs`)

**职责**：mdBook Renderer trait 实现、配置解析、后端调度、后处理触发

```rust
// 核心 trait 实现
pub struct PdfRenderer;

impl Renderer for PdfRenderer {
    fn name(&self) -> &str { "pdf" }
    fn render(&self, ctx: &RenderContext) -> Result<(), Error> {
        run_pdf(ctx)
    }
}
```

**实现要点**：

```
┌─────────────────────────────────────────────────────────────┐
│                    run_pdf() 执行流程                         │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  1. 解析配置 ──► ctx.config.get("output.pdf")               │
│     │              → toml::Value → serde_json → PdfOptions  │
│     ▼                                                       │
│  2. 定位 print.html                                        │
│     │   html_dir = destination/../html/                     │
│     │   print_html = html_dir/print.html                    │
│     ▼                                                       │
│  3. 提取章节路径                                            │
│     │   ctx.book.iter() → filter BookItem::Chapter          │
│     │   → ch.path → Vec<String>                            │
│     ▼                                                       │
│  4. 提取书籍元数据                                          │
│     │   title, authors, description, language               │
│     ▼                                                       │
│  5. 后端调度                                                │
│     │   match cfg.backend:                                  │
│     │     "chrome-cli" → render_chrome()                    │
│     │     _           → render_chrome_cdp()                 │
│     │                   失败 → render_chrome() (回退)       │
│     ▼                                                       │
│  6. 后处理                                                  │
│     │   pdf_outline::postprocess_pdf()                      │
│     │   (书签 + 元数据)                                     │
│     ▼                                                       │
│  7. 输出日志                                                │
│       PDF 大小、路径                                         │
└─────────────────────────────────────────────────────────────┘
```

### 4.2 模块二：Chrome CDP 后端 (`pdf_chrome_cdp.rs`)

**职责**：通过 Chrome DevTools Protocol 的 `Page.printToPDF` 方法生成 PDF [[17]]

**核心依赖**：`chromiumoxide` crate（Rust CDP 客户端）[[18]]

```rust
// 异步渲染核心
async fn render_chrome_cdp_async(
    temp_html: &Path,
    output_pdf: &Path,
    cfg: &PdfOptions,
    use_css: bool,
) -> Result<(), anyhow::Error> {
    // 1. 构建浏览器配置
    let browser_config = BrowserConfig::builder()
        .chrome_executable(chrome_path)
        .no_sandbox()
        .build()?;

    // 2. 启动浏览器
    let (mut browser, mut handler) = Browser::launch(browser_config).await?;

    // 3. 打开页面
    let file_url = Url::from_file_path(temp_html)?;
    let page = browser.new_page(file_url.as_str()).await?;
    page.wait_for_navigation().await?;

    // 4. (可选) CSS 注入页眉/页脚
    if use_css {
        page.evaluate(build_css_injection_js(cfg)).await?;
    }

    // 5. 调用 Page.printToPDF
    let params = build_pdf_params(cfg);
    let pdf_data = page.pdf(params).await?;

    // 6. 写入文件
    std::fs::write(output_pdf, &pdf_data)?;

    // 7. 关闭浏览器
    browser.close().await?;
    Ok(())
}
```

**CDP 参数构建**：

```rust
fn build_pdf_params(cfg: &PdfOptions) -> PrintToPdfParams {
    let hf_enabled = cfg.header_footer_enabled();
    let use_cdp_hf = hf_enabled && cfg.use_native_header_footer;

    PrintToPdfParams {
        display_header_footer: use_cdp_hf.then_some(true),
        header_template: use_cdp_hf
            .then(|| cfg.header_template.clone())
            .filter(|s| !s.is_empty()),
        footer_template: use_cdp_hf
            .then(|| cfg.footer_template.clone())
            .filter(|s| !s.is_empty()),
        landscape: cfg.landscape.then_some(true),
        print_background: Some(cfg.print_background),
        scale: scaled_or_none(cfg.scale),
        paper_width: Some(cfg.paper_width),
        paper_height: Some(cfg.paper_height),
        margin_top: Some(cfg.margin_top),
        margin_bottom: Some(cfg.margin_bottom),
        margin_left: Some(cfg.margin_left),
        margin_right: Some(cfg.margin_right),
        page_ranges: (!cfg.page_range.is_empty())
            .then(|| cfg.page_range.clone()),
        prefer_css_page_size: Some(cfg.prefer_css_page_size),
        transfer_mode: None,
        generate_tagged_pdf: Some(cfg.generate_tagged_pdf),
        generate_document_outline: None, // 由后处理负责
    }
}
```

### 4.3 模块三：HTML 预处理 (`pdf_html_preprocess.rs`)

**职责**：在 HTML 送入 Chrome 前进行多维度预处理

**核心依赖**：`scraper` crate（基于 Servo html5ever + selectors）[[33]]

```
┌─────────────────────────────────────────────────────────────────┐
│                  HTML 预处理流水线                                │
│                                                                 │
│  原始 print.html                                                │
│       │                                                         │
│       ▼                                                         │
│  ┌─────────────────┐                                           │
│  │ 1. fix_links()  │  相对链接 → 绝对 URL                      │
│  └────────┬────────┘  (仅当 static-site-url 非空)               │
│           ▼                                                     │
│  ┌─────────────────────┐                                       │
│  │ 2. inject_toc_fix() │  插入 <a id="chapter-id"> 锚点        │
│  └────────┬────────────┘  Chrome 将其转为 PDF 命名目标          │
│           ▼                                                     │
│  ┌──────────────────────┐                                      │
│  │ 3. inject_print_css()│  @media print 分页控制 CSS           │
│  └────────┬─────────────┘                                      │
│           ▼                                                     │
│  ┌──────────────────────┐                                      │
│  │ 4. inject_font_css() │  CJK 字体回退 CSS                    │
│  └────────┬─────────────┘                                      │
│           ▼                                                     │
│  ┌─────────────────┐                                           │
│  │ 5. inject_js()  │  展开 <details>、MathJax 挂钩、哨兵       │
│  └────────┬────────┘                                           │
│           ▼                                                     │
│  预处理后的 HTML → 写入临时文件 → Chrome 加载                    │
└─────────────────────────────────────────────────────────────────┘
```

**各步骤实现细节**：

#### 4.3.1 ToC 锚点注入

```rust
/// 章节路径 → PDF 命名目标 ID
/// "chapter/01-setup.md" → "chapter-01-setup"
pub fn chapter_path_to_id(path: &str) -> String {
    let mut base = path.to_string();
    if base.ends_with(".md") { base.truncate(base.len() - 3); }
    base.replace(['/', '\\'], "-").to_ascii_lowercase()
}

/// 在 </body> 前插入隐藏锚点
fn inject_toc_fix(html: &str, chapter_paths: &[String]) -> String {
    let mut toc_fix = String::from("<div style=\"display: none\">");
    for path in chapter_paths {
        let id = chapter_path_to_id(path);
        toc_fix.push_str(&format!("<a id=\"{}\"></a>", id));
    }
    toc_fix.push_str("</div>");
    // 插入到 </body> 前
    insert_before(html, "</body>", &toc_fix)
}
```

#### 4.3.2 JS 注入（内容加载哨兵）

```rust
fn inject_js(html: &str) -> (String, bool) {
    let script = r#"
    <script type='text/javascript'>
    let markAllContentHasLoadedForPrinting = () =>
        window.setTimeout(() => {
            let p = document.createElement('div');
            p.setAttribute('id',
              'content-has-all-loaded-for-mdbook-pdf-generation');
            document.body.appendChild(p);
        }, 100);

    window.addEventListener('load', () => {
        // 展开所有 <details>
        for (let d of document.getElementsByTagName('details'))
            d.open = true;
        // MathJax 完成挂钩
        try {
            MathJax.Hub.Register.StartupHook(
              'End', markAllContentHasLoadedForPrinting);
        } catch (e) {
            markAllContentHasLoadedForPrinting();
        }
    });
    </script>"#;
    (insert_before(html, "</body>", script), true)
}
```

#### 4.3.3 链接修正

```rust
fn fix_links(html: &str, base_url: &str) -> String {
    if base_url.is_empty() { return html.to_string(); }
    let base_url = base_url.trim_end_matches('/');
    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href]").unwrap();

    let mut replacements: Vec<(String, String)> = Vec::new();
    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            if let Some(fixed) = fix_single_link(href, base_url) {
                replacements.push((href.to_string(), fixed));
            }
        }
    }
    // 字符串替换 href 属性值
    let mut result = html.to_string();
    for (old, new) in &replacements {
        result = result.replace(
            &format!("href=\"{}\"", old),
            &format!("href=\"{}\"", new),
        );
    }
    result
}
```

### 4.4 模块四：PDF 后处理 (`pdf_outline.rs`)

**职责**：书签生成 + PDF 元数据写入

**核心依赖**：`lopdf` crate（PDF 文档操作）[[26]]

```
┌─────────────────────────────────────────────────────────────────┐
│                  PDF 后处理流程                                   │
│                                                                 │
│  Chrome 生成的 PDF (无书签)                                      │
│       │                                                         │
│       ▼                                                         │
│  ┌──────────────────────────────────────────┐                   │
│  │ 1. Document::load(pdf_path)              │                   │
│  └────────────────┬─────────────────────────┘                   │
│                   ▼                                              │
│  ┌──────────────────────────────────────────┐                   │
│  │ 2. extract_bookmark_entries(print.html)  │                   │
│  │    解析 .header 锚点 → h1-h6 标题        │                   │
│  │    输出: Vec<BEntry { level, title, id }>│                   │
│  └────────────────┬─────────────────────────┘                   │
│                   ▼                                              │
│  ┌──────────────────────────────────────────┐                   │
│  │ 3. resolve_dests(doc)                    │                   │
│  │    解析 PDF 命名目标 → ObjectId 映射      │                   │
│  │    /Dests 或 /Names/Dests                │                   │
│  └────────────────┬─────────────────────────┘                   │
│                   ▼                                              │
│  ┌──────────────────────────────────────────┐                   │
│  │ 4. add_bookmarks(doc, entries)           │                   │
│  │    构建层级书签树                         │                   │
│  │    doc.add_bookmark() + build_outline()  │                   │
│  └────────────────┬─────────────────────────┘                   │
│                   ▼                                              │
│  ┌──────────────────────────────────────────┐                   │
│  │ 5. add_metadata(doc, ...)                │                   │
│  │    /Title /Author /Subject /Lang         │                   │
│  └────────────────┬─────────────────────────┘                   │
│                   ▼                                              │
│  ┌──────────────────────────────────────────┐                   │
│  │ 6. doc.save(pdf_path)                    │                   │
│  └──────────────────────────────────────────┘                   │
│                                                                 │
│  输出: 带书签和元数据的 PDF                                      │
└─────────────────────────────────────────────────────────────────┘
```

**书签提取算法**：

```rust
fn extract_bookmark_entries(html: &str) -> Result<Vec<BEntry>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(".header")?;
    let mut entries = Vec::new();

    for el in doc.select(&sel) {
        // 1. 获取 href="#some-id"
        let href = el.value().attr("href")
            .filter(|h| h.starts_with('#'))
            .map(|h| &h[1..]);

        // 2. 查找对应 id 的元素
        let target = doc.select(&id_selector(href)).next();

        // 3. 确认是 h1-h6，提取层级和标题
        let level = tag_to_level(target.value().name());
        let title = target.text().collect::<String>().trim().to_string();

        entries.push(BEntry { level, title, dest_name: href });
    }
    Ok(entries)
}
```

**命名目标解析**（支持两种 PDF 存储格式）：

```rust
fn resolve_dests(doc: &Document) -> Result<IndexMap<String, ObjectId>> {
    let catalog = doc.catalog()?;
    let mut map = IndexMap::new();

    // 方式 1: Catalog → /Dests (直接字典)
    if let Ok(dests_obj) = catalog.get(b"Dests") {
        for (name_bytes, dest_obj) in dests_dict.iter() {
            let name = percent_decode_to_lossy(name_bytes);
            let page_id = dest_obj_to_page_id(doc, dest_obj);
            map.insert(name, page_id);
        }
    }

    // 方式 2: Catalog → /Names → /Dests (名称树)
    if let Ok(names_obj) = catalog.get(b"Names") {
        // 递归遍历名称树...
    }

    Ok(map)
}
```

### 4.5 模块五：Chrome CLI 后端 (`render_chrome`)

**职责**：降级方案，通过命令行参数调用 Chrome headless

```rust
fn render_chrome(
    print_html: &Path,
    output_pdf: &Path,
    cfg: &PdfOptions,
) -> Result<()> {
    let chrome = resolve_chrome(&cfg.browser_binary_path)?;

    let mut cmd = Command::new(&chrome);
    cmd.arg("--headless")
       .arg("--disable-gpu")
       .arg("--no-sandbox")
       .arg(format!("--print-to-pdf={}", output_pdf.display()))
       .arg(format!("--print-to-pdf-no-header={}",
           !cfg.header_footer_enabled()));

    if cfg.landscape { cmd.arg("--landscape"); }

    // 页眉/页脚模板（仅原生模式）
    if cfg.header_footer_enabled() && cfg.use_native_header_footer {
        if !cfg.header_template.is_empty() {
            cmd.arg(format!("--header-template={}", cfg.header_template));
        }
        if !cfg.footer_template.is_empty() {
            cmd.arg(format!("--footer-template={}", cfg.footer_template));
        }
    }

    cmd.arg(temp_html_path);
    cmd.output()?;
    Ok(())
}
```

### 4.6 模块六：浏览器探测 (`resolve_chrome`)

```
┌─────────────────────────────────────────────────────────────────┐
│                  Chrome 探测优先级                                │
│                                                                 │
│  优先级 1: 环境变量 CHROME                                       │
│       │   std::env::var("CHROME") → 验证 is_file()              │
│       ▼                                                         │
│  优先级 2: 配置路径 browser-binary-path                          │
│       │   PathBuf::from(cfg.browser_binary_path)                │
│       ▼                                                         │
│  优先级 3: 平台自动检测                                          │
│       │                                                         │
│       ├── Linux:                                                │
│       │   google-chrome-stable → google-chrome                  │
│       │   → chromium-browser → chromium                         │
│       │                                                         │
│       ├── macOS:                                                │
│       │   /Applications/Google Chrome.app/...                   │
│       │   /Applications/Chromium.app/...                        │
│       │                                                         │
│       └── Windows:                                              │
│           C:\Program Files\Google\Chrome\...\chrome.exe         │
│           C:\Program Files (x86)\...\chrome.exe                 │
│                                                                 │
│  检测方式: Path::is_file() || which(name)                        │
└─────────────────────────────────────────────────────────────────┘
```

---

## 5. 数据流图

### 5.1 完整数据流

```
┌──────────┐     stdin      ┌──────────────┐
│  mdbook  │ ──── JSON ───► │  PdfRenderer │
│  build   │  RenderContext │  .render()   │
└──────────┘                └──────┬───────┘
                                   │
                    ┌──────────────┼──────────────┐
                    │              │              │
                    ▼              ▼              ▼
             ┌───────────┐ ┌──────────┐ ┌────────────┐
             │ book.toml │ │ SUMMARY  │ │ print.html │
             │ [output.  │ │ .md      │ │ (HTML后端  │
             │  pdf]     │ │ (章节)   │ │  已生成)   │
             └─────┬─────┘ └────┬─────┘ └─────┬──────┘
                   │            │              │
                   ▼            ▼              │
             ┌──────────────────────┐          │
             │    PdfOptions        │          │
             │    (配置结构体)      │          │
             └──────────┬───────────┘          │
                        │                      │
                        ▼                      ▼
             ┌──────────────────────────────────────┐
             │         HTML 预处理                   │
             │  fix_links → inject_toc_fix →        │
             │  inject_print_css → inject_font_css  │
             │  → inject_js                         │
             └──────────────────┬───────────────────┘
                                │
                                ▼
             ┌──────────────────────────────────────┐
             │     临时文件: print_pdf.html          │
             └──────────────────┬───────────────────┘
                                │
                    ┌───────────┴───────────┐
                    │                       │
                    ▼                       ▼
         ┌─────────────────┐    ┌─────────────────┐
         │  Chrome CDP     │    │  Chrome CLI     │
         │  (WebSocket)    │    │  (subprocess)   │
         │                 │    │                 │
         │  Page.printToPDF│    │  --print-to-pdf │
         └────────┬────────┘    └────────┬────────┘
                  │                      │
                  └──────────┬───────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │   原始 PDF (无书签)  │
                  └──────────┬──────────┘
                             │
                             ▼
                  ┌─────────────────────────────────┐
                  │        PDF 后处理                │
                  │                                 │
                  │  print.html ──► 提取标题结构    │
                  │  PDF ──► 解析命名目标           │
                  │  合并 ──► 构建书签树            │
                  │  写入 ──► 元数据                │
                  └──────────────────┬──────────────┘
                                     │
                                     ▼
                  ┌─────────────────────────────────┐
                  │   最终 PDF (带书签+元数据)       │
                  │   output/pdf/output.pdf         │
                  └─────────────────────────────────┘
```

### 5.2 书签生成数据流

```
print.html (原始)                    Chrome 生成的 PDF
─────────────────                    ─────────────────

<h1 id="ch1">
  <a class="header"    ──解析──►    /Catalog
    href="#ch1">                      /Dests
  </a>                                  /ch1 → [page_ref, 0, 0]
</h1>                                   /ch1-s1 → [page_ref, 0, 0]
                                        /ch2 → [page_ref, 0, 0]
<h2 id="ch1-s1">
  <a class="header"                  /Pages
    href="#ch1-s1">                    [page1, page2, page3...]
  </a>
</h2>

         │                                    │
         ▼                                    ▼
┌─────────────────┐              ┌─────────────────────┐
│ Vec<BEntry>     │              │ IndexMap<String,    │
│                 │              │        ObjectId>    │
│ {level:1,       │              │                     │
│  title:"Ch1",   │              │ "ch1" → (3,0)      │
│  dest:"ch1"}    │              │ "ch1-s1" → (3,0)   │
│ {level:2,       │              │ "ch2" → (5,0)      │
│  title:"S1",    │              └──────────┬──────────┘
│  dest:"ch1-s1"} │                         │
└────────┬────────┘                         │
         │                                  │
         └──────────────┬───────────────────┘
                        │
                        ▼
         ┌──────────────────────────────┐
         │      书签树构建               │
         │                              │
         │  doc.add_bookmark(           │
         │    Bookmark::new(            │
         │      "Ch1",                  │
         │      color,                  │
         │      0,                      │
         │      page_id=(3,0)           │
         │    ),                        │
         │    parent=None               │
         │  )                           │
         │                              │
         │  doc.add_bookmark(           │
         │    Bookmark::new("S1",...),  │
         │    parent=Some(ch1_id)       │
         │  )                           │
         │                              │
         │  doc.build_outline()         │
         └──────────────────────────────┘
```

---

## 6. 核心依赖库

### 6.1 Cargo.toml 依赖清单

| Crate | 版本 | 用途 | 对应模块 |
|-------|------|------|---------|
| `mdbook` | `0.4.x` | mdBook 插件框架（Renderer/Preprocessor trait） | 全局 |
| `chromiumoxide` | `0.7+` | Chrome CDP 协议客户端 [[17]] | pdf_chrome_cdp |
| `chromiumoxide-cdp` | `0.7+` | CDP 类型定义（PrintToPdfParams 等） | pdf_chrome_cdp |
| `tokio` | `1.x` | 异步运行时（CDP 通信） | pdf_chrome_cdp |
| `futures` | `0.3` | 异步流处理 | pdf_chrome_cdp |
| `lopdf` | `0.34+` | PDF 文档操作（书签/元数据）[[26]] | pdf_outline |
| `scraper` | `0.20+` | HTML 解析 + CSS 选择器 [[33]] | pdf_html_preprocess, pdf_outline |
| `serde` + `serde_json` | `1.x` | 配置反序列化 | pdf |
| `toml` | `0.8+` | book.toml 解析 | pdf |
| `anyhow` | `1.x` | 错误处理 | 全局 |
| `log` | `0.4` | 日志 | 全局 |
| `indexmap` | `2.x` | 有序 HashMap（命名目标映射） | pdf_outline |
| `url` | `2.x` | 文件路径 → file:// URL | pdf_chrome_cdp |
| `semver` | `1.x` | 版本兼容性检查 | utils |

### 6.2 依赖关系图

```
                    ┌─────────────┐
                    │   mdbook    │ (插件框架)
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
              ▼            ▼            ▼
     ┌──────────────┐ ┌────────┐ ┌──────────┐
     │chromiumoxide │ │ lopdf  │ │ scraper  │
     │  (CDP客户端) │ │(PDF操作)│ │(HTML解析)│
     └──────┬───────┘ └────────┘ └──────────┘
            │
     ┌──────┴───────┐
     │    tokio     │ (异步运行时)
     │   futures    │
     └──────────────┘
```

---

## 7. 配置系统设计

### 7.1 完整配置结构体

```rust
#[derive(serde::Deserialize, Debug, Clone)]
#[serde(default, rename_all = "kebab-case")]
pub struct PdfOptions {
    // ── 后端选择 ──
    pub backend: String,              // "chrome" | "chrome-cli"
    pub browser_binary_path: String,  // Chrome 路径
    pub trying_times: u32,            // 重试次数

    // ── 页面几何（英寸） ──
    pub paper_width: f64,             // 默认 8.5
    pub paper_height: f64,            // 默认 11.0
    pub landscape: bool,              // 默认 false
    pub margin_top: f64,              // 默认 1.0
    pub margin_bottom: f64,           // 默认 1.0
    pub margin_left: f64,             // 默认 1.0
    pub margin_right: f64,            // 默认 1.0
    pub scale: f64,                   // 默认 1.0
    pub prefer_css_page_size: bool,   // 默认 false

    // ── 页眉/页脚 ──
    pub no_header: Option<bool>,      // 强制禁用开关
    pub display_header_footer: bool,  // 默认 false
    pub use_native_header_footer: bool, // 默认 false
    pub css_header_footer: bool,      // 默认 true
    pub header_height: f64,           // 默认 0.7 (CSS模式)
    pub footer_height: f64,           // 默认 0.6 (CSS模式)
    pub header_template: String,      // HTML 模板
    pub footer_template: String,      // HTML 模板

    // ── 内容控制 ──
    pub print_background: bool,       // 默认 true
    pub page_range: String,           // 默认 ""
    pub ignore_invalid_page_ranges: bool, // 默认 false
    pub generate_document_outline: bool,  // 默认 true
    pub generate_tagged_pdf: bool,    // 默认 true

    // ── 链接修复 ──
    pub static_site_url: String,      // 默认 ""
}
```

### 7.2 book.toml 配置示例

```toml
[book]
title = "My Book"
authors = ["Author Name"]
language = "zh"
src = "./src"

[build]
build-dir = "output"

[output.html]
[output.html.print]
enable = true

[output.pdf]
command = "mdbook-plugins pdf"
optional = true
backend = "chrome"

# 页面几何
paper-width = 8
paper-height = 10
margin-top = 0.5
margin-bottom = 1.0
margin-left = 0.5
margin-right = 0.5

# 页眉/页脚
no-header = false
display-header-footer = true
use-native-header-footer = true
css-header-footer = false
header-template = """
<div style='width:100%;text-align:center;font-size:10px;'>
  <span class='title'></span>
</div>"""
footer-template = """
<div style='width:100%;text-align:center;font-size:10px;'>
  <span class='pageNumber'></span>
</div>"""

# 内容控制
print-background = true
generate-document-outline = true
generate-tagged-pdf = true
```

---

## 8. 关键算法与实现细节

### 8.1 书签层级构建算法

```
输入: entries = [
  {level:1, title:"Chapter 1", dest:"ch1"},
  {level:2, title:"Section 1.1", dest:"ch1-s1"},
  {level:2, title:"Section 1.2", dest:"ch1-s2"},
  {level:1, title:"Chapter 2", dest:"ch2"},
  {level:3, title:"Sub 2.1.1", dest:"ch2-s1-s1"},
]

算法: 使用栈维护父级关系
─────────────────────────────────────────

stack = []

Entry(level=1, "Chapter 1"):
  find_parent(stack, 1) → None (根级)
  add_bookmark("Chapter 1", parent=None) → id=1
  stack = [(1, 1)]

Entry(level=2, "Section 1.1"):
  find_parent(stack, 2) → Some(1) (level 1 < 2)
  add_bookmark("Section 1.1", parent=Some(1)) → id=2
  stack = [(1, 1), (2, 2)]

Entry(level=2, "Section 1.2"):
  find_parent(stack, 2) → pop (2,2), then Some(1)
  add_bookmark("Section 1.2", parent=Some(1)) → id=3
  stack = [(1, 1), (2, 3)]

Entry(level=1, "Chapter 2"):
  find_parent(stack, 1) → pop all, None
  add_bookmark("Chapter 2", parent=None) → id=4
  stack = [(1, 4)]

Entry(level=3, "Sub 2.1.1"):
  find_parent(stack, 3) → Some(4) (level 1 < 3)
  add_bookmark("Sub 2.1.1", parent=Some(4)) → id=5
  stack = [(1, 4), (3, 5)]

结果书签树:
├── Chapter 1
│   ├── Section 1.1
│   └── Section 1.2
└── Chapter 2
    └── Sub 2.1.1
```

### 8.2 命名目标 URL 解码

```rust
/// Chrome 在 PDF 中使用 URL 编码存储中文命名目标
/// 例: %E5%BC%95%E8%A8%80 → "引言"
fn percent_decode_to_lossy(bytes: &[u8]) -> String {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                decoded.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).to_string()
}
```

### 8.3 CSS 注入页眉/页脚原理

```
┌─────────────────────────────────────────────────────────────┐
│  CSS 注入模式工作原理                                        │
│                                                             │
│  1. 注入 <style> 到 <head>:                                 │
│     @media print {                                          │
│       .pf-h, .pf-f {                                       │
│         display: block;                                     │
│         position: fixed;                                    │
│         left: 0; right: 0; width: 100%;                    │
│         z-index: 10000;                                    │
│       }                                                     │
│       .pf-h { top: 0; height: {header_height}in; }         │
│       .pf-f { bottom: 0; height: {footer_height}in; }      │
│     }                                                       │
│     @page {                                                 │
│       margin: {mt+hh}in {mr}in {mb+fh}in {ml}in;          │
│     }                                                       │
│                                                             │
│  2. 注入 <div class="pf-h"> 和 <div class="pf-f">          │
│     到 <body> 末尾                                          │
│                                                             │
│  3. 边距补偿:                                               │
│     实际 margin-top = 用户 margin-top + header-height       │
│     实际 margin-bottom = 用户 margin-bottom + footer-height │
│                                                             │
│  视觉效果:                                                  │
│  ┌─────────────────────────────────┐                        │
│  │  [页眉区域 - position:fixed]    │ ← header_height       │
│  ├─────────────────────────────────┤                        │
│  │                                 │                        │
│  │         正文内容区域             │ ← 用户 margin         │
│  │                                 │                        │
│  ├─────────────────────────────────┤                        │
│  │  [页脚区域 - position:fixed]    │ ← footer_height       │
│  └─────────────────────────────────┘                        │
└─────────────────────────────────────────────────────────────┘
```

### 8.4 页面索引回退分配算法

当 PDF 中没有命名目标时，按比例将书签分配到页面：

```rust
// 后备方案：按比例分配页面
let idx = ((i * pages.len()) / entries.len()).min(pages.len() - 1);
pages[idx]

// 示例: 10 个书签, 5 页
// 书签 0 → page[0*5/10] = page[0]
// 书签 1 → page[1*5/10] = page[0]
// 书签 2 → page[2*5/10] = page[1]
// ...
// 书签 9 → page[9*5/10] = page[4]
```

---

## 9. 错误处理与重试机制

### 9.1 错误处理策略

```
┌─────────────────────────────────────────────────────────────────┐
│                    错误处理层级                                   │
│                                                                 │
│  Level 1: 致命错误 (中断构建)                                    │
│  ├── print.html 不存在 → warn + return Ok(())                   │
│  ├── Chrome 未找到 → warn + return Ok(())                       │
│  └── PDF 文件无法写入 → Err (中断)                              │
│                                                                 │
│  Level 2: 可恢复错误 (降级/重试)                                 │
│  ├── CDP 连接失败 → 回退到 CLI 模式                             │
│  ├── CDP 超时 → 重试 (trying_times 次)                          │
│  └── 单次 CDP 尝试失败 → 等待 500ms → 重试                     │
│                                                                 │
│  Level 3: 非致命错误 (警告继续)                                  │
│  ├── 后处理书签失败 → warn, PDF 仍输出                          │
│  ├── 元数据写入失败 → warn, PDF 仍输出                          │
│  └── 命名目标解析失败 → 回退到页面索引分配                       │
└─────────────────────────────────────────────────────────────────┘
```

### 9.2 重试机制实现

```rust
let max_attempts = std::cmp::max(1, cfg.trying_times) as usize;

for attempt in 1..=max_attempts {
    match rt.block_on(async {
        render_chrome_cdp_async(&temp_html, output_pdf, cfg, use_css).await
    }) {
        Ok(()) => return Ok(()),
        Err(e) if attempt < max_attempts => {
            log::warn!("attempt {} failed: {}. Retrying...", attempt, e);
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(e) => return Err(e),
    }
}
```

### 9.3 CDP → CLI 回退逻辑

```rust
match cfg.backend.as_str() {
    "chrome-cli" => {
        // 强制 CLI 模式
        render_chrome(&print_html, &output_pdf, &cfg, &html_dir)?;
    }
    _ => {
        // 默认: 尝试 CDP，失败回退 CLI
        let html_content = std::fs::read_to_string(&print_html)?;
        match render_chrome_cdp(&html_content, &output_pdf, ...) {
            Ok(()) => {}
            Err(e) => {
                log::warn!("CDP failed ({}). Falling back to CLI.", e);
                render_chrome(&print_html, &output_pdf, &cfg, &html_dir)?;
            }
        }
    }
}
```

---

## 10. 测试策略

### 10.1 测试矩阵

| 模块 | 测试类型 | 测试内容 |
|------|---------|---------|
| `pdf_html_preprocess` | 单元测试 | `chapter_path_to_id` 转换正确性 |
| `pdf_html_preprocess` | 单元测试 | `inject_toc_fix` 锚点插入位置 |
| `pdf_html_preprocess` | 单元测试 | `fix_links` 各类链接处理（相对/绝对/锚点/协议） |
| `pdf_html_preprocess` | 单元测试 | `inject_print_css` 插入位置（有/无 `</head>`） |
| `pdf_html_preprocess` | 集成测试 | 完整预处理流水线 |
| `pdf_outline` | 单元测试 | `extract_bookmark_entries` 标题解析 |
| `pdf_outline` | 单元测试 | `find_parent` 栈操作 |
| `pdf_outline` | 单元测试 | `percent_decode_to_lossy` URL 解码 |
| `pdf_chrome_cdp` | 单元测试 | `build_pdf_params` 参数映射 |
| `pdf_chrome_cdp` | 单元测试 | `build_css_injection_js` JS 生成 |
| `pdf_chrome_cdp` | 单元测试 | `scaled_or_none` 边界值 |
| `pdf` | 单元测试 | `PdfOptions` 默认值 |
| `pdf` | 单元测试 | `header_footer_enabled` 优先级逻辑 |
| 全局 | 集成测试 | 端到端 PDF 生成（需 Chrome 环境） |

### 10.2 测试示例

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chapter_path_to_id() {
        assert_eq!(chapter_path_to_id("intro.md"), "intro");
        assert_eq!(chapter_path_to_id("chapter/01-setup.md"), "chapter-01-setup");
    }

    #[test]
    fn test_extract_basic() {
        let html = r#"<html><body>
            <a class="header" href="#intro"></a>
            <h1 id="intro">Introduction</h1>
            <a class="header" href="#setup"></a>
            <h2 id="setup">Setup Guide</h2>
        </body></html>"#;
        let es = extract_bookmark_entries(html).unwrap();
        assert_eq!(es.len(), 2);
        assert_eq!(es[0].level, 1);
        assert_eq!(es[0].title, "Introduction");
        assert_eq!(es[1].level, 2);
    }

    #[test]
    fn test_cdp_native_mode() {
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        cfg.use_native_header_footer = true;
        cfg.header_template = "<span class='title'></span>".into();
        let params = build_pdf_params(&cfg);
        assert_eq!(params.display_header_footer, Some(true));
        assert!(params.header_template.is_some());
    }

    #[test]
    fn test_no_header_overrides() {
        let mut cfg = PdfOptions::default();
        cfg.display_header_footer = true;
        cfg.no_header = Some(true);
        assert!(!cfg.header_footer_enabled());
    }

    #[test]
    fn test_fix_relative_link() {
        let html = r#"<a href="page.html">link</a>"#;
        let result = fix_links(html, "https://example.com/book");
        assert!(result.contains(
            r#"href="https://example.com/book/page.html""#));
    }

    #[test]
    fn test_fix_anchor_skipped() {
        let html = r#"<a href="#section">link</a>"#;
        let result = fix_links(html, "https://example.com/book");
        assert_eq!(result, html);
    }
}
```

---

## 11. 项目目录结构

```
mdbook-pdf-rs/
├── Cargo.toml                    # 项目配置 + 依赖
├── src/
│   ├── main.rs                   # 入口：CLI 参数解析 + 子命令分发
│   ├── lib.rs                    # 库入口：模块声明
│   ├── utils.rs                  # 通用工具（run_renderer, run_preprocessor）
│   ├── pdf/
│   │   ├── mod.rs                # PDF 模块入口
│   │   ├── pdf.rs                # PdfRenderer + run_pdf + 配置 + CLI 后端
│   │   ├── pdf_chrome_cdp.rs     # Chrome CDP 后端（异步）
│   │   ├── pdf_html_preprocess.rs # HTML 预处理流水线
│   │   └── pdf_outline.rs        # PDF 后处理（书签 + 元数据）
│   └── (其他插件模块...)
├── tests/
│   ├── integration_test.rs       # 端到端集成测试
│   └── fixtures/
│       ├── sample_print.html     # 测试用 print.html
│       └── sample.pdf            # 测试用 PDF
├── book.toml                     # 示例配置
└── README.md
```

---

## 附录 A：实施路线图

```
Phase 1 (MVP) ─────────────────────────────────────────────
  [ ] 项目骨架 + Cargo.toml
  [ ] PdfOptions 配置解析
  [ ] Chrome 探测 (resolve_chrome)
  [ ] Chrome CLI 后端 (render_chrome)
  [ ] 基本 @page CSS 注入
  目标: 能生成最基础的 PDF

Phase 2 (CDP) ─────────────────────────────────────────────
  [ ] chromiumoxide 集成
  [ ] CDP 异步渲染 (render_chrome_cdp_async)
  [ ] PrintToPdfParams 完整映射
  [ ] CDP → CLI 自动回退
  [ ] 重试机制
  目标: CDP 模式可用，支持所有 printToPDF 参数

Phase 3 (预处理) ──────────────────────────────────────────
  [ ] ToC 锚点注入 (inject_toc_fix)
  [ ] 打印 CSS 注入 (inject_print_css)
  [ ] 字体 CSS 注入 (inject_font_css)
  [ ] JS 注入 (inject_js)
  [ ] 链接修正 (fix_links)
  目标: HTML 预处理完整

Phase 4 (后处理) ──────────────────────────────────────────
  [ ] 书签提取 (extract_bookmark_entries)
  [ ] 命名目标解析 (resolve_dests)
  [ ] 书签树构建 (add_bookmarks)
  [ ] PDF 元数据 (add_metadata)
  目标: PDF 带完整书签和元数据

Phase 5 (页眉/页脚) ───────────────────────────────────────
  [ ] CDP 原生模式 (headerTemplate/footerTemplate)
  [ ] CSS 注入模式 (position:fixed)
  [ ] 四种组合模式
  [ ] 模板变量替换
  目标: 页眉/页脚完整支持

Phase 6 (打磨) ────────────────────────────────────────────
  [ ] 完整测试覆盖
  [ ] 错误处理优化
  [ ] 日志完善
  [ ] 文档
  [ ] CI/CD
  目标: 生产可用
```

---

## 附录 B：关键技术决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| CDP 客户端 | `chromiumoxide` | 纯 Rust 实现，类型安全，活跃维护 [[17]] |
| PDF 操作 | `lopdf` | 纯 Rust，支持书签/元数据操作 [[26]] |
| HTML 解析 | `scraper` | 基于 Servo 引擎，CSS 选择器支持完整 [[33]] |
| 异步运行时 | `tokio` | chromiumoxide 依赖，生态成熟 |
| 书签生成 | 后处理而非 CDP | 更精确控制层级结构，不依赖 Chrome 版本 |
| 命名目标 | `<a id>` 注入 | Chrome 只为有 id 的元素创建命名目标 |
| 配置格式 | kebab-case | 与 mdbook 生态一致 |
| 错误处理 | `anyhow` + 分级 | 致命/可恢复/非致命三级处理 |

---

## 附录 C：mdbook 插件通信协议

```
mdbook build
    │
    │  启动子进程: mdbook-plugins pdf
    │
    │  stdin ──► RenderContext (JSON)
    │            {
    │              "version": "0.4.x",
    │              "root": "/path/to/book",
    │              "book": { "sections": [...] },
    │              "config": { "book": {...}, "output": {...} },
    │              "destination": "/path/to/output/pdf"
    │            }
    │
    │  stdout ◄── (无输出，直接写文件)
    │
    │  exit code: 0 = 成功, 非0 = 失败
    ▼
output/pdf/output.pdf
```

---

> **文档版本**: v1.0
> **最后更新**: 2026-07-19
> **参考项目**: [HollowMan6/mdbook-pdf](https://github.com/HollowMan6/mdbook-pdf.git) [[10]]
> **CDP 规范**: [Chrome DevTools Protocol - Page.printToPDF](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-printToPDF) [[2]]