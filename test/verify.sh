#!/bin/bash
# mdbook-plugins 验证脚本
# 用法: ./verify.sh [--full]
#   --full  执行完整 mdbook build 测试

set -e
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PLUGIN_BIN="$REPO_DIR/target/release/mdbook-plugins"
TEST_DIR="$REPO_DIR/test"
BIN_DIR="$TEST_DIR/bin"
ABS_BIN_DIR="$BIN_DIR"
PASS=0
FAIL=0

green() { echo -e "\033[32m$1\033[0m"; }
red()   { echo -e "\033[31m$1\033[0m"; }
pass()  { PASS=$((PASS+1)); green "  ✅ $1"; }
fail()  { FAIL=$((FAIL+1)); red "  ❌ $1"; }

echo "=========================================="
echo "  mdbook-plugins 验证脚本"
echo "=========================================="
echo ""

# 检查构建产物
if [ ! -f "$PLUGIN_BIN" ]; then
    red "❌ 二进制不存在，请先构建: cargo build --release"
    exit 1
fi
pass "mdbook-plugins binary ($(du -h "$PLUGIN_BIN" | cut -f1))"

# 重新部署到 test/bin/
cp "$PLUGIN_BIN" "$BIN_DIR/mdbook-plugins"
ALL_PLUGINS="mdbook-admonish mdbook-alerts mdbook-echarts mdbook-emojicodes
    mdbook-embedify mdbook-katex mdbook-kroki-preprocessor mdbook-langtabs
    mdbook-mermaid mdbook-pikchr mdbook-svgbob mdbook-toc mdbook-wavedrom-rs
    mdbook-asciidoc mdbook-linkcheck mdbook-office mdbook-pdf"

for name in $ALL_PLUGINS; do
    ln -sf mdbook-plugins "$BIN_DIR/$name" 2>/dev/null || true
done
pass "符号链接已创建 ($(echo $ALL_PLUGINS | wc -w) 个)"

# tests: supports 协议
echo ""
echo "--- supports 协议 ---"
for plugin in mdbook-admonish mdbook-alerts mdbook-toc mdbook-mermaid mdbook-katex; do
    if PATH="$BIN_DIR:$PATH" "$BIN_DIR/$plugin" supports html 2>/dev/null; then
        pass "$plugin supports html"
    else
        fail "$plugin supports html"
    fi
    if PATH="$BIN_DIR:$PATH" "$BIN_DIR/$plugin" supports not-supported 2>/dev/null; then
        fail "$plugin supports not-supported (应返回 1)"
    else
        pass "$plugin rejects not-supported"
    fi
done

# tests: 路由正确性
echo ""
echo "--- 路由测试 ---"
for plugin in mdbook-admonish mdbook-toc mdbook-katex mdbook-pdf; do
    OUTPUT=$(PATH="$BIN_DIR:$PATH" "$BIN_DIR/$plugin" 2>&1 <<< "" || true)
    if echo "$OUTPUT" | grep -q "mdbook-plugins ($plugin)"; then
        pass "$plugin 路由正确"
    else
        fail "$plugin 路由异常: $OUTPUT"
    fi
done

# 完整构建测试 (--full)
if [ "${1:-}" = "--full" ]; then
    echo ""
    echo "--- mdbook build ---"
    cd "$TEST_DIR"

    # 备份并运行构建
    cp book.toml book.toml.bak

    if PATH="$ABS_BIN_DIR:$PATH" "mdbook" build 2>&1 | tail -10; then
        pass "mdbook build 成功"
        if [ -f "test_books/index.html" ]; then
            pass "输出文件存在"
        fi
    else
        fail "mdbook build 失败"
    fi

    # 恢复 book.toml
    mv book.toml.bak book.toml 2>/dev/null || true
    cd "$REPO_DIR"
fi

echo ""
echo "=========================================="
if [ $FAIL -eq 0 ]; then
    green "  全部 $PASS 项测试通过！"
else
    red "  $PASS 通过, $FAIL 失败"
fi
echo "=========================================="
exit $FAIL
