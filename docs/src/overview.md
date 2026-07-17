# mdbook-plugins 文档

**mdbook-plugins** 是一个将 17 个独立 mdbook 插件合并为单一二进制文件的 Rust 项目。
通过 `argv[0]` 符号链接路由机制，一个二进制文件即可替代所有原独立插件，
大幅减少磁盘占用并简化部署。

## 核心指标

| 指标 | 数值 |
|------|------|
| 插件总数 | 17（13 预处理器 + 4 渲染器）|
| 二进制大小 | ~7.5 MB（Release, LTO + strip）|
| 原独立二进制总大小 | ~115 MB |
| 磁盘节省 | ~94% |
| 代码行数 | ~1,700 行 Rust |
| 外部 C 代码 | ~8,200 行（pikchr.c，内置）|
| Rust 版本 | edition 2021 |

## 设计目标

1. **单二进制分发** — 所有插件编译进一个可执行文件
2. **零配置路由** — 通过符号链接名称自动识别插件类型
3. **模块化扩展** — 新增插件只需创建模块 + 注册路由
4. **完全兼容** — 遵循 mdbook 标准预处理器/渲染器协议
5. **独立编译** — 所有源码（含 C 依赖）内置在项目内

## 快速开始

```bash
# 构建
cd /home/kuanghl/workspace/rpp/repo/mdbook-plugins
cargo build --release

# 测试路由
./target/release/mdbook-plugins mdbook-admonish supports html
echo $?   # 应输出 0

# 部署到 mdbook-demo
cp target/release/mdbook-plugins ../mdbook-demo/bin/
cd ../mdbook-demo/bin
for name in mdbook-admonish mdbook-alerts mdbook-toc mdbook-katex \
    mdbook-mermaid mdbook-echarts mdbook-emojicodes mdbook-svgbob; do
    ln -sf mdbook-plugins "$name"
done

# 验证构建
cd ../mdbook-demo
PATH="./bin:$PATH" mdbook build
```
