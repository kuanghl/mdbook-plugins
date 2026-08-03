# crates.io 发布指南

本文档说明如何将 `mdbook-plugins` 发布到 [crates.io](https://crates.io)。

## 前置条件

- 拥有 [crates.io](https://crates.io) 账号
- 本地已安装 Rust 工具链（cargo）
- 包名 `mdbook-plugins` 未被占用（发布前可先在 https://crates.io/crates/mdbook-plugins 确认，当前可用）

## 一次性准备：登录 crates.io

发布需要 crates.io API token：

1. 登录 crates.io，进入 **Account Settings → API Tokens**。
2. 点击 **New Token**，创建后复制 token（token 只显示一次）。
3. 在终端执行：

```bash
cd /home/kuanghl/workspace/rpp/mdbook-plugins
echo "你的Token" | cargo login
# cargo login --registry 你的registry名 可以指向使用的镜像
# 凭证保存在 ~/.cargo/credentials，token 不要泄露到仓库或文档
```

## 检查发布元数据

`cargo publish` 前需确认 `Cargo.toml` 的 `[package]` 段已包含：

```toml
[package]
name = "mdbook-plugins"           # 包名，一经发布不可更改
version = "0.1.0"                 # 版本号，发布后不可复用
edition = "2021"
description = "单二进制多插件支持的 mdbook 插件集合"
license = "MIT"                   # 必填
repository = "https://github.com/kuanghl/mdbook-plugins"
readme = "README.md"              # crates.io 页面展示的 README
keywords = ["mdbook", "plugin", "preprocessor", "renderer", "documentation"]
categories = ["development-tools", "text-processing"]
```

> 注意：包名发布后**无法修改**；版本号发布后**无法复用**。首次发布前请确认 crates.io 上未被占用。

## 本地验证（发布前）

### 1. 打包检查

```bash
cd /home/kuanghl/workspace/rpp/mdbook-plugins

# 查看将要打包的文件列表
cargo package --list

# 打包并编译验证（无错误即通过）
cargo package
```

打包内容由 `Cargo.toml` 的 `include` 白名单决定：仅 `src/`、`build.rs`、`vendor/`、`assets/`、`Cargo.toml`、`Cargo.lock`、`README.md`。
`test/`、`test-mini/`、`docs/` 及构建产物（target/ 等）不进入 `.crate` 包——这也保证包体积在 crates.io 的 10 MiB 限制以内。

### 2. 预演发布（dry-run）

```bash
cargo publish --dry-run
```

不真正上传，但会执行 crates.io 的完整校验（包名、元数据、依赖可用性等）。

## 正式发布

```bash
cargo publish
```

上传成功后终端会输出版本信息。版本一旦发布**不可撤回**，需要撤回时使用 yank（见下文）。

## 发布预编译二进制（GitHub tag 触发）

除 crates.io 源码包外，项目通过 GitHub Actions 自动构建各平台**预编译二进制**（免编译安装）。
工作流 `.github/workflows/release.yml` 在推送 `v*` tag 时触发，在 Windows / Linux / macOS 三平台编译全功能版（含 TikZ/Typst）并上传到 [GitHub Releases](https://github.com/kuanghl/mdbook-plugins/releases)。

### 1. 准备（仅首次）

确保工作流文件已提交到仓库：

```bash
git add .github/workflows/release.yml
git commit -m "ci: 添加预编译二进制发布工作流"
git push
```

### 2. 发布新版本（完整流程）

```bash
# ① 更新版本号（patch，需 cargo-edit：cargo install cargo-edit）
cargo set-version --bump patch
# 或手动修改 Cargo.toml 的 version 字段

# ② 提交并推送代码
git add Cargo.toml Cargo.lock
git commit -m "release: v0.1.4"
git push

# ③ 发布到 crates.io（源码安装）
cargo publish

# ④ 打 tag 并推送，触发 CI 自动构建预编译二进制
git tag v0.1.4
git push origin v0.1.4
```

> 建议 tag 名（`v0.1.4`）与 `Cargo.toml` 的 version（`0.1.4`）保持一致，便于对照。

### 3. 等待与验证

1. GitHub → **Actions** 页签查看 Release 工作流（3 平台并行；全功能版含 tectonic/typst，编译较慢，约 30–60 分钟）。
2. 完成后在 **Releases** 页签看到 3 个产物：

   | 平台 | 产物 |
   |------|------|
   | Windows | `mdbook-plugins-x86_64-pc-windows-msvc.zip`（单 exe，静态链接无 dll） |
   | Linux | `mdbook-plugins-x86_64-unknown-linux-gnu.tar.gz` |
   | macOS | `mdbook-plugins-aarch64-apple-darwin.tar.gz` |

3. 免编译安装验证：

```powershell
cargo install cargo-binstall
cargo binstall mdbook-plugins
mdbook-plugins --version
```

### 注意事项

- Windows 版为 vcpkg 静态链接单 exe，仅依赖系统 MSVC 运行时（Windows 10/11 一般自带）。
- macOS 产物为 **arm64**（Apple Silicon）；Intel Mac 用户请源码编译。
- 某平台 CI 失败时，修复 workflow 后需**删除并重新推送同名 tag** 才能重新触发：

```bash
git tag -d v0.1.4
git push origin :refs/tags/v0.1.4
git tag v0.1.4
git push origin v0.1.4
```

## 发布后验证

1. **crates.io 页面**：https://crates.io/crates/mdbook-plugins
2. **API 文档**：https://docs.rs/mdbook-plugins（发布后几分钟自动生成）
3. **安装测试**：

```bash
cargo install mdbook-plugins
mdbook-plugins --help
mdbook-plugins --version

# 卸载指令
cargo uninstall mdbook-plugins
```

> docs.rs 默认按 `default` features 构建文档，其中 `pre-tikz`（tectonic）、`pre-typst` 依赖较重，构建可能超时。
> 若 docs.rs 文档构建失败，可限制其构建范围（不影响 crates.io 发布）：

```toml
[package.metadata.docs.rs]
features = ["pre-toc", "pre-katex", "pre-pikchr", "ren-pdf"]
```

## 更新版本并再次发布

```bash
# 递增版本号（patch，需 cargo-edit：cargo install cargo-edit）
cargo set-version --bump patch
# 或手动修改 Cargo.toml 的 version 字段

# 重新发布到 crates.io
cargo publish

# 打新 tag 并推送，同步触发预编译二进制构建（见「发布预编译二进制」）
git commit -am "release: v0.1.5"
git push
git tag v0.1.5
git push origin v0.1.5
```

## 撤回版本（yank）

发布错误版本时，可以 yank（标记为不可用），但无法删除：

```bash
cargo yank --version 0.1.0
# 撤销 yank
cargo yank --version 0.1.0 --undo
```

> yank 需要 token 具有对应权限；若创建 token 时只勾选了 `publish-new`，需要另建 token 或在 crates.io 上操作。

## 注意事项

- **token 保密**：API token 等同密码，不要提交到 git 或写入任何文档。
- **不可逆操作**：包名、版本号一经发布不可修改/复用；发布前务必先执行 `cargo package` 与 `cargo publish --dry-run`。
- **体积**：完整构建约 64 MB（含 tectonic / typst 引擎），`cargo install` 编译时间较长属正常现象；用户可按需使用 `--no-default-features --features ...` 选择轻量功能。
