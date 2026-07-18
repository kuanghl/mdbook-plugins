# 插件参考

## 预处理插件

### mdbook-admonish

**模块**: `src/preprocessors/admonish.rs` (118 行)

**功能**: 将 fenced code block 语法 ```` ```admonish warning ``` ```` 渲染为 Material Design 风格的提示框。

**核心算法**:
```
输入 Markdown → pulldown-cmark AST 解析
  → 遍历 Event 流，识别 CodeBlock(Fenced("admonish ..."))
  → 提取类型名 (warning/note/tip 等) 和正文内容
  → 拼接为 <div class="mdbook-admonish-{type}"> HTML
  → 在 </head> 前注入 CSS 样式
  → 输出处理后的 Markdown
```

**关键代码**:
```rust
// 使用 pulldown-cmark 解析代码块，避免 regex 的限制
Event::Start(Tag::CodeBlock(kind)) => {
    let info = match &kind {
        CodeBlockKind::Fenced(s) => s.to_string(),
        CodeBlockKind::Indented => String::new(),
    };
    if info.trim().starts_with("admonish ") || info.trim() == "admonish" {
        in_admonish = true;
        admonish_type = info.trim_start_matches("admonish").trim().to_string();
        if admonish_type.is_empty() { admonish_type = "note".to_string(); }
        continue;
    }
}
```

**与原始插件的差异**:
- 原始插件使用 pulldown-cmark 完整 AST 解析（含递归 admonish 嵌套），本模块使用简化的状态机遍历
- 原始插件支持 `install` 子命令自动配置 book.toml，本模块仅处理预处理核心逻辑
- CSS 改为 `include_str!` 内嵌，无需外部文件

**注意**: 不支持 admonish 嵌套（admonish 内再放 admonish）。

---

### mdbook-alerts

**模块**: `src/preprocessors/alerts.rs` (94 行)

**功能**: 将 GitHub Flavored Markdown 的 `> [!NOTE]` 语法转换为 HTML alert 块。

**核心算法**:
```
输入 Markdown → 正则匹配 `> [!KIND]\n> body`
  → 解析 type (note/warning/tip/important/caution)
  → 替换为 <div class="mdbook-alerts mdbook-alerts-{type}"> 模板
  → 在 <head> 前注入 CSS 样式
  → 输出
```

**关键代码**:
```rust
// 注意：regex 不支持 lookbehind，此处用 ^ 配合 multi-line 模式
static RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^> \[!(?P<kind>[^\]]+)\]\s*$(?P<body>(?:\n>.*)*)")
        .expect("failed to parse regex")
});
```

**与原始插件的差异**:
- 原始插件使用 `rust-embed` 加载 CSS/模板，本模块使用 `include_str!`
- 模板和 CSS 内容与原始插件完全一致

**注意**: 正则使用 `(?m)` multi-line 模式而非默认模式。模板占位符为 `{kind}` 和 `{body}`，通过 `str::replace` 替换。

---

### mdbook-echarts

**模块**: `src/preprocessors/echarts.rs` (69 行)

**功能**: 将 ```` ```echarts ``` ```` 或 `{% echarts %}...{% endecharts %}` 中的 JSON 配置转换为 ECharts 初始化代码。

**核心算法**:
```regex
# 匹配两种语法格式
```echarts\n(.*?)```        # fenced code block
{%\s*echarts\s*%}(.*?){%\s*endecharts\s*%}  # 模板标签
# → 替换为:
<div id="echarts-{uuid}">...</div>
<script>echarts.init(...).setOption({json})</script>
```

**关键代码**:
```rust
let id = Uuid::new_v4().to_string().replace('-', "");
// 每个图表实例分配唯一 DOM ID，防止多图表冲突
```

**注意**: 需要前端页面加载 `echarts.min.js`。仅支持 `renderer = ["html"]`。

---

### mdbook-emojicodes

**模块**: `src/preprocessors/emojicodes.rs` (60 行)

**功能**: 将 `:cat:`、`:rocket:` 等 emoji shortcode 替换为 Unicode 表情符号 🐱 🚀。

**核心算法**:
```
逐行扫描 Markdown
  → 检测代码块围栏 (```/~~~)，进出时翻转 inside_code_block 标志
  → 在代码块外，用正则 :([^:\s]*?): 匹配 shortcode
  → 通过 emojis::get_by_shortcode() 查询标准 emoji
  → 替换为 Unicode 字符
```

**与原始插件的差异**:
- 原始插件支持自定义 SVG emoji（从 `src/custom_emojis/` 目录加载），本模块仅支持标准 emoji
- 使用 `emojis` crate 替代手动映射表

**注意**: 代码块内的 shortcode 不会被替换。不支持嵌套的代码块标记（```` ``` ```` 内再放 ```` ``` ````）。

---

### mdbook-embedify

**模块**: `src/preprocessors/embedify.rs` (96 行)

**功能**: 将 `{% youtube id %}`、`{% codepen user slug %}` 等标签转换为嵌入式 HTML。

**支持的应用**:
| 标签 | 语法 | 输出 |
|------|------|------|
| YouTube | `{% youtube VIDEO_ID %}` | `<iframe>` 嵌入播放器 |
| CodePen | `{% codepen USER SLUG %}` | `<iframe>` 嵌入 Pen |
| Giscus | `{% giscus repo=X repo-id=Y category=Z %}` | Giscus 评论脚本 |

**核心算法**: 三个正则分别匹配三种标签格式，替换为对应的 HTML 模板。

**与原始插件的差异**:
- 原始插件使用 `pest` 解析器 + 模板引擎 + 自定义模板目录，功能完备但代码复杂（~500 行）
- 本模块仅实现三种最常用的嵌入类型，使用简单正则匹配

**注意**: 如需更多嵌入类型，可按相同模式扩展正则和替换函数。

---

### mdbook-katex

**模块**: `src/preprocessors/katex.rs` (114 行)

**功能**: 将 `$...$`（行内）和 `$$...$$`（块级）LaTeX 数学公式转换为 KaTeX HTML。

**核心算法**:
```
字符流逐个扫描（手动解析，不使用 regex）
  → 遇到 $ 时检查下一个字符：
     → 如果是 $ → 块级模式：查找闭合 $$，提取内容
     → 如果不是 $ → 行内模式：查找闭合 $，提取内容
  → 内容用 html_escape() 转义特殊字符
  → 包裹在 <span class="katex"> 或 <div class="katex-display"> 中
```

**关键代码**:
```rust
// 手动字符解析避免 regex 不支持 lookahead/lookbehind 的限制
while let Some(c) = chars.next() {
    if c == '$' {
        if chars.peek() == Some(&'$') {
            // $$ 块级公式
            // ...
        } else {
            // $ 行内公式
            // ...
        }
    }
}
```

**为什么不用 regex**: Rust `regex` crate 不支持 lookahead `(?!)` 和 lookbehind `(?<!)`，
无法区分 `$$`（块级）和 `$`（行内）。手动解析可以精确控制定界符匹配逻辑。

**与原始插件的差异**:
- 原始插件使用 `katex` crate（Rust 绑定）进行服务端预渲染，本模块仅转义为 HTML 占位符
- 实际 KaTeX 渲染由前端的 `katex.min.js` + CSS 完成

---

### mdbook-kroki-preprocessor

**模块**: `src/preprocessors/kroki.rs` (97 行)

**功能**: 将 ```` ```kroki-graphviz ``` ```` 代码块发送到 [kroki.io](https://kroki.io) 渲染为内联 SVG。

**核心算法**:
```
正则匹配 ```kroki-{type}\n{body}```
  → 对 body 进行 deflate 压缩
  → base64url 编码
  → 构造 Kroki URL: {endpoint}/{type}/svg/{encoded}
  → 用 reqwest::blocking::get 请求
  → 替换为 SVG 内容
```

**关键代码**:
```rust
// Kroki 要求 deflate 压缩 + base64url 无填充编码
fn base64_encode(input: &str) -> String {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(input.as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();
    Engine::encode(&URL_SAFE_NO_PAD, &compressed)
}
```

**环境变量**: `KROKI_ENDPOINT`（默认 `https://kroki.io`）

**注意**: 需要网络访问 Kroki 服务。离线环境无法使用。

---

### mdbook-langtabs

**模块**: `src/preprocessors/langtabs.rs` (110 行)

**功能**: 将 `<!-- langtabs-start -->` 和 `<!-- langtabs-end -->` 包裹的多代码块区域转换为 HTML Tab 切换 UI。

**算法**:
```
1. 在内容中循环查找 <!-- langtabs-start --> 和 <!-- langtabs-end -->
2. 提取中间区域
3. 用正则 ```(\w+)\n(.*?)``` 解析各个代码块（语言 + 代码）
4. 生成 HTML:
   <div class="langtabs">
     <ul class="langtabs-tabs">
       <li class="langtabs-tab active">rust</li>
       <li class="langtabs-tab">python</li>
     </ul>
     <div class="langtabs-panel active">...</div>
     <div class="langtabs-panel">...</div>
   </div>
```

**注意**: 需要前端配合 CSS/JS 实现 Tab 切换交互。Tab ID 使用自增数字，确保页面内唯一。

---

### mdbook-mermaid

**模块**: `src/preprocessors/mermaid.rs` (43 行)

**功能**: 将 ```` ```mermaid ``` ```` 代码块替换为 `<pre class="mermaid">` 标签。

**核心算法**:
```rust
// 最简单的实现：一行正则
let re = Regex::new(r"(?ms)```mermaid\s*\n(.*?)```").unwrap();
// 替换为:
format!("<pre class=\"mermaid\">\n{}</pre>\n", diagram.trim())
```

**注意**: 需要前端加载 `mermaid.min.js` + `mermaid-init.js`。实际渲染在浏览器端完成。

---

### mdbook-pikchr

**模块**: `src/preprocessors/pikchr.rs` (156 行)

**功能**: 将 ```` ```pikchr ``` ```` 中的 Pikchr 脚本通过 C FFI 编译为 SVG。

**依赖**: `vendor/pikchr.c`（~8,200 行 C 代码）+ `build.rs` + `libc` crate

**核心算法**:
```
pulldown-cmark 解析 → 识别 ```pikchr 代码块
  → 提取 Pikchr 脚本文本
  → 通过 extern "C" 调用 pikchr() 函数
  → 回调函数接收生成的 SVG 字符串
  → 包裹在 <div class="pikchr-wrapper"> 中输出
```

**FFI 声明**:
```rust
extern "C" {
    fn pikchr(
        pikchr: *const c_char,    // Pikchr 脚本
        flags: c_uint,            // 渲染标志
        result_cb: extern "C" fn(*const c_char, c_int, *mut c_void),  // 成功回调
        err_cb: ... ,             // 错误回调
        user_data: *mut c_void,   // 用户数据（用于收集输出）
    ) -> c_int;                   // 返回 0 失败，非 0 成功
}
```

**注意**: 需要系统安装 C 编译器（gcc/clang）。`build.rs` 在构建时自动编译 `vendor/pikchr.c`。

---

### mdbook-svgbob

**模块**: `src/preprocessors/svgbob.rs` (112 行)

**功能**: 将 ```` ```bob ``` ```` 中的 ASCII 艺术图通过 `svgbob` crate 渲染为 SVG。

**核心算法**:
```
pulldown-cmark 解析 → 识别 ```bob 代码块
  → 提取 ASCII art 文本
  → svgbob::to_svg_with_settings(ascii, &settings) 转换为 SVG
  → 直接输出 SVG 内容
```

**依赖**: `svgbob = "0.7"`、`svg = "0.18"`

**注意**: svgbob 0.7 选择 `Default::default()` settings 即可满足大部分场景。
渲染失败时回退为 `<pre><code>` 显示原始 ASCII。

---

### mdbook-toc

**模块**: `src/preprocessors/toc.rs` (92 行)

**功能**: 查找 `<!-- toc -->` 标记，自动扫描其后的标题（Heading）生成 Markdown 格式目录。

**核心算法**:
```
1. 在 Markdown 中查找 <!-- toc --> 标记位置
2. 截取标记之后的内容
3. 用 pulldown-cmark 解析 Heading 事件
4. 收集 level 1-4 的标题文本和 slug
5. 生成缩进格式的目录列表：
     * [标题](#slug)
       * [子标题](#sub-slug)
6. 插入到 <!-- toc --> 标记之后
```

**slug 生成**:
```rust
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .fold(String::new(), |mut acc, c| {
            if c == '-' && acc.ends_with('-') { /* 去重 */ }
            else { acc.push(c); }
            acc
        })
        .trim_matches('-')
        .to_string()
}
```

**注意**: 生成的目录使用 Markdown 格式（`* [text](#slug)`），mdbook 会在 HTML 输出中自动渲染。

---

### mdbook-wavedrom-rs

**模块**: `src/preprocessors/wavedrom.rs` (88 行)

**功能**: 将 ```` ```wavedrom ``` ```` 中的 WaveJSON 代码包裹在 `<pre class="wavedrom">` 标签中。

**核心算法**: 与 mermaid 类似，使用 pulldown-cmark 识别 `wavedrom` 代码块，
提取内容后包裹在专属标签中，由前端 `wavedrom.min.js` 渲染为时序图 SVG。

**注意**: 需要前端加载 `wavedrom.min.js` + `wavedrom.css.js`。

---

## 渲染插件

### mdbook-asciidoc

**模块**: `src/renderers/asciidoc.rs` (136 行)

**功能**: 将 mdbook 内容输出为 AsciiDoc 格式文件。

**核心算法**:
```
stdin → RenderContext → Book
  → pulldown-cmark 解析 Markdown 事件流
  → 逐个映射为 AsciiDoc 语法：
     Heading → ===== 标题
     CodeBlock → [source,lang] ---- 代码 ----
     List → * 列表项
     Link → url[text]
     Table → | 表格
  → 写入文件系统
```

**Markdown → AsciiDoc 映射表**:
| Markdown | AsciiDoc |
|----------|----------|
| `# 标题` | `= 标题` |
| `## 标题` | `== 标题` |
| ` ```rust\ncode\n``` ` | `[source,rust]\n----\ncode\n----` |
| `[text](url)` | `url[text]` |
| `* item` | `* item` |
| 换行 | ` +\n`（硬换行）|

---

### mdbook-linkcheck

**模块**: `src/renderers/linkcheck.rs` (137 行)

**功能**: 遍历书中所有链接，使用 tokio 异步检查链接有效性。

**核心算法**:
```
1. 遍历 Book 的所有章节
2. 用正则 \[([^\]]+)\]\(([^)]+)\) 提取所有链接
3. 过滤：跳过 # 锚点和 mailto: 链接
4. 对每个 URL 发起 HEAD 请求
5. 如果返回 405（Method Not Allowed），回退为 GET 请求
6. 收集所有失败链接，记录日志
```

**异步并发**:
```rust
let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(10))
    .build()?;

let handles: Vec<_> = links.iter().map(|url| {
    let client = client.clone();
    let url = url.clone();
    tokio::spawn(async move {
        // 并发检查每个链接
        client.head(&url).send().await...
    })
}).collect();

let results = futures::future::join_all(handles).await;
```

**注意**: 检查失败仅记录日志，不中断构建。支持 `http://` 和 `https://` 协议，
跳过相对路径和 `file://` 链接。

---

### mdbook-office

**模块**: `src/renderers/office.rs` (约 300 行)

**功能**: 通过 `office_oxide` 库将 mdbook 内容生成为 DOCX/XLSX/PPTX 文档。

**算法流程**:
```
stdin → RenderContext
  → 从 config 解析输出格式 (docx/xlsx/pptx)
  → 递归收集所有章节内容（含子章节）
  → clean_text() 清洗 HTML 标签，保留 Markdown 结构
  → 合并为一个 Markdown 字符串
  → office_oxide::create_from_markdown() 生成 Office 文档
  → 写入 destination 目录
```

**HTML 清理算法**:
```rust
// 逐个字符扫描，跳过 HTML 标签和 script/style 块
// 遇到块级闭合标签时插入换行符
// 合并连续空白行（最多 2 行）
```

**注意**: 图标渲染（Mermaid/ECharts/KaTeX）需要 headless Chrome 截图，
简化版本跳过此步骤。

---

### mdbook-pdf

**模块**: `src/renderers/pdf.rs` + `src/renderers/pdf_chrome_cdp.rs` + `src/renderers/pdf_html_preprocess.rs`

**功能**: PDF 渲染器，通过 Chrome DevTools Protocol 生成高质量 PDF。

**核心特性**:
- **四模式页眉/页脚**：`use-native-header-footer`（CDP 原生模板）与 `css-header-footer`（CSS 注入）两个正交开关，组合实现4种模式：仅 CSS 注入、仅 CDP 原生、CDP 原生+CSS 叠加、无页眉/页脚
- **HTML 预处理**：使用 `scraper` DOM 解析，执行链接修正 + 增强分页 CSS 注入
- **自动重试**：`trying-times` 参数控制失败重试，每次重试重新启动 Chrome 实例
- **Chrome 版本检测**：通过 `chrome --version` 自动检测版本，>= 125 时自动启用原生模式
- **回退机制**：CDP 后端失败时自动回退 CLI `--headless --print-to-pdf` 模式

**book.toml 完整配置**:
```toml
[output.pdf]
command = "mdbook-plugins pdf"
optional = true

# ── 后端与重试 ──
# backend = "chrome"                     # "chrome"（默认）或 "chrome-cli"
# trying-times = 1                        # CDP 失败重试次数
# browser-binary-path = ""                # Chrome 路径，留空自动探测（支持 CHROME 环境变量）

# ── 页面几何（单位：英寸） ──
# paper-width = 8.5
# paper-height = 11.0
# landscape = false
# margin-top = 1.0
# margin-bottom = 1.0
# margin-left = 1.0
# margin-right = 1.0
# scale = 1.0                             # 全局缩放
# prefer-css-page-size = false

# ── 页眉/页脚 ──
# display-header-footer = false           # 是否启用（被 no_header=true 覆盖）
# use-native-header-footer = false        # true=CDP原生模板, false=CSS固定定位
# css-header-footer = true                # CSS注入独立开关（与use-native-header-footer正交）
# header-height = 0.7                     # 固定定位模式：页眉高度（英寸）
# footer-height = 0.6                     # 固定定位模式：页脚高度（英寸）
# header-template = ""                    # 支持 class='date/title/pageNumber/totalPages'
# footer-template = ""
# no_header = false                       # 设为 true 则无条件禁用所有页眉/页脚

# ── 内容控制 ──
# print-background = true
# page-range = ""                         # 如 "1-5,8,11-13"
# ignore-invalid-page-ranges = false
# generate-document-outline = true        # PDF 书签大纲
# generate-tagged-pdf = true              # 无障碍标签

# ── 链接修复 ──
# static-site-url = ""                    # 相对链接转绝对 URL 的基准
```

**架构流程**:
```
parse_config() → PdfOptions
     │
     ▼
preprocess_html(html, cfg):             [pdf_html_preprocess.rs]
  ├─ 1. fix_links()        (scraper DOM 解析)
  ├─ 2. inject_print_css() (h1新页/代码保护/孤行控制)
  └─ ▶ 返回处理后的 HTML
     │
     ▼
render_chrome_cdp(html, output, cfg):   [pdf_chrome_cdp.rs]
  ├─ 重试循环 (1..=cfg.trying_times)
  │    └─ render_chrome_cdp_async()
  │         ├─ chrome --version → 版本检测
  │         ├─ determine_mode() → None / CSS / Native + CSS / Native
  │         ├─ 启动 Chrome (no_sandbox)
  │         ├─ 导航到临时 HTML
  │         ├─ page.evaluate(注入JS)  [仅 Fixed 模式]
  │         ├─ page.pdf(params)
  │         └─ 写入 output.pdf
  └─ 清理临时文件
```

**Chrome 查找顺序**:
1. 环境变量 `CHROME`
2. `book.toml` 的 `browser-binary-path`
3. PATH 搜索：`google-chrome-stable` → `chromium-browser` → `chromium`

**页眉/页脚模式选择**:

`css-header-footer` 与 `use-native-header-footer` 是正交的独立开关，实现4种组合：

| `display-header-footer` | `no_header` | `use-native-header-footer` | `css-header-footer` | 模式 | 说明 |
|---|---|---|---|---|---|
| `false` 或未设置 | — | — | — | None | 无页眉/页脚 |
| `true` | `true` | — | — | None | `no_header` 覆盖，无页眉/页脚 |
| `true` | `false`/未设置 | `false`（默认） | `true`（默认） | CSS 注入 | 仅 CSS 注入 |
| `true` | `false`/未设置 | `false`（默认） | `false` | None | 无页眉/页脚 |
| `true` | `false`/未设置 | `true` | `true` | Native + CSS | CDP 原生 + CSS 注入叠加 |
| `true` | `false`/未设置 | `true` | `false` | Native | 仅 CDP 原生 |

> **注意**：某些 Chrome 版本（如 v150）存在 `displayHeaderFooter` CDP 参数被忽略的 bug，导致 Chrome 默认页眉/页脚与自定义模板同时渲染。如果遇到此问题，建议升级 Chrome 或改用纯 CSS 注入模式（`use-native-header-footer=false` + `css-header-footer=true`）。

**关键 CSS 规则**（固定定位模式）：
- `@page` 边距 = 用户边距 + 页眉/页脚高度（补偿）
- `.pf-header` / `.pf-footer`：`position: fixed`，`z-index: 10000`
- CDP `PrintToPdfParams.margin` 设为 `0`，完全由 CSS 控制边距

**分页保护 CSS**（两种模式均注入）：
- `h1 { page-break-before: always; }` — 每章新页
- `pre, code, table, img, svg { page-break-inside: avoid; }` — 代码块不断页
- `p, li { widows: 2; orphans: 2; }` — 孤行控制
- `.mermaid, .echarts { page-break-inside: avoid; }` — 图表保护

---

## 未合并的插件说明

下列插件的源文件存在于 `prj_mdbook/` 目录或 `bin/` 目录中，
但由于技术原因未合并到本项目：

| 插件 | 原二进制大小 | 未合并原因 | 替代方案 |
|------|------------|-----------|---------|
| **mdbook-catppuccin** | 2.5 MB | **非 Rust 项目**——SCSS 主题样式仓库，通过 `sass` 编译为 CSS。没有 Cargo.toml。 | 通过 book.toml 的 `additional-css` 引入编译后的 CSS |
| **mdbook-whichlang** | 5.3 MB (JS) | **非 Rust 项目**——Deno/TypeScript 前端插件，产物是 `whichlang.js` + CSS。 | 通过 book.toml 的 `additional-css` + `additional-js` 引入 |
| **mdbook-image-viewer** | 3.7 MB | **已合并**（`src/preprocessors/image_viewer.rs`）| 已包含在 mdbook-plugins 中 |
| **mdbook-pandoc** | 21.8 MB | **代码量大**（~5,000 行，17+ 源文件）+ **依赖外部 pandoc 工具**。需额外安装 pandoc。 | 保留独立二进制 |
| **mdbook-xgettext** | 17.9 MB | **属于 i18n 工具链**——Google 维护的 `mdbook-i18n-helpers` 项目的一部分。核心库 lib.rs 69KB，含 4 个子 crate。 | 保留独立二进制（与 mdbook-gettext 配合使用）|
| **mdbook-gettext** | 17.8 MB | 同上，属于 `mdbook-i18n-helpers` 项目。提取/应用翻译的完整工具链。 | 保留独立二进制 |
| **mdbook-i18n** | 22.5 MB | 同上，i18n 辅助工具。 | 保留独立二进制 |
| **mdbook-i18n-normalize** | 17.6 MB | 同上，PO/POT 文件标准化工具。 | 保留独立二进制 |
| **cloud-translate** | 21.8 MB | 同上，GCP Cloud Translate API 自动翻译 PO 文件。依赖 tokio + google-cloud-auth。 | 保留独立二进制 |

这些插件的原始二进制完整保留在 `bin/backup_*/` 目录中，可通过 PATH 正常使用。

---

## 特性清单

| 特性名 | 对应插件 | 类型 | 额外依赖 |
|--------|---------|------|---------|
| `pre-alerts` | mdbook-alerts | 预处理器 | — |
| `pre-emojicodes` | mdbook-emojicodes | 预处理器 | — |
| `pre-toc` | mdbook-toc | 预处理器 | — |
| `pre-echarts` | mdbook-echarts | 预处理器 | — |
| `pre-langtabs` | mdbook-langtabs | 预处理器 | — |
| `pre-mermaid` | mdbook-mermaid | 预处理器 | — |
| `pre-katex` | mdbook-katex | 预处理器 | — |
| `pre-admonish` | mdbook-admonish | 预处理器 | — |
| `pre-svgbob` | mdbook-svgbob | 预处理器 | svgbob, svg |
| `pre-pikchr` | mdbook-pikchr | 预处理器 | libc, C 编译器, vendor/pikchr.c |
| `pre-kroki` | mdbook-kroki-preprocessor | 预处理器 | reqwest, flate2, base64 |
| `pre-embedify` | mdbook-embedify | 预处理器 | — |
| `pre-wavedrom` | mdbook-wavedrom-rs | 预处理器 | — |
| `ren-asciidoc` | mdbook-asciidoc | 渲染器 | — |
| `ren-linkcheck` | mdbook-linkcheck | 渲染器 | tokio, reqwest |
| `ren-office` | mdbook-office | 渲染器 | office_oxide |
| `ren-pdf` | mdbook-pdf | 渲染器 | chromiumoxide + scraper（Chrome CDP 双模式 + HTML 预处理）|
