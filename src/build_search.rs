//! mdbook-build-search — 中文搜索索引构建工具
//!
//! 在 mdbook build 之后运行，扫描生成的 HTML 文件，
//! 用中文 bigram 分词构建搜索索引，替换默认的 elasticlunr 搜索。
//!
//! 用法: mdbook-plugins build-search <html-dir>

use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// 搜索文档
#[derive(Debug, Serialize)]
struct SearchDoc {
    id: usize,
    url: String,
    title: String,
    body: String,
}

/// 搜索索引（输出为 JSON）
#[derive(Debug, Serialize)]
struct SearchIndex {
    documents: Vec<SearchDoc>,
    tokens: HashMap<String, Vec<usize>>,
}

/// 判断字符是否为 CJK 中文字符
fn is_cjk(c: char) -> bool {
    let code = c as u32;
    (code >= 0x4E00 && code <= 0x9FFF)
        || (code >= 0x3400 && code <= 0x4DBF)
        || (code >= 0xF900 && code <= 0xFAFF)
}

/// CJK bigram 分词器
///
/// - 中文连续字符：拆分为相邻双字（bigram），单字也保留
/// - 英文/数字：按非字母数字分割
fn cjk_bigram_tokenize(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cjk_buf: Vec<char> = Vec::new(); // 积累连续中文字符
    let mut latin_buf = String::new();       // 积累连续英文/数字

    for ch in text.chars() {
        if is_cjk(ch) {
            // flush latin buffer
            if !latin_buf.is_empty() {
                tokens.push(latin_buf.clone());
                latin_buf.clear();
            }
            cjk_buf.push(ch);
        } else if ch.is_ascii_alphanumeric() {
            // flush CJK buffer
            if !cjk_buf.is_empty() {
                tokens.extend(cjk_bigram_from_buf(&cjk_buf));
                cjk_buf.clear();
            }
            latin_buf.push(ch);
        } else {
            // 分隔符：flush both
            if !cjk_buf.is_empty() {
                tokens.extend(cjk_bigram_from_buf(&cjk_buf));
                cjk_buf.clear();
            }
            if !latin_buf.is_empty() {
                tokens.push(latin_buf.clone());
                latin_buf.clear();
            }
        }
    }
    // flush remaining
    if !cjk_buf.is_empty() {
        tokens.extend(cjk_bigram_from_buf(&cjk_buf));
    }
    if !latin_buf.is_empty() {
        tokens.push(latin_buf);
    }

    // 去重（同一段文本中同一个 token 出现多次只计一次）
    tokens.sort();
    tokens.dedup();
    tokens
}

/// 对 CJK 字符序列生成 bigram（相邻双字），同时保留单字
fn cjk_bigram_from_buf(buf: &[char]) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    // unigram: 每个单字
    for &c in buf {
        result.push(c.to_string());
    }
    // bigram: 相邻双字
    for i in 0..buf.len().saturating_sub(1) {
        let mut s = String::with_capacity(6);
        s.push(buf[i]);
        s.push(buf[i + 1]);
        result.push(s);
    }
    result
}

/// 从 HTML 中提取文本（去除标签）
fn extract_text(html: &str) -> String {
    // 去除 script/style/svg 标签及其内容
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let re_svg = Regex::new(r"(?is)<svg[^>]*>.*?</svg>").unwrap();
    let step1 = re_script.replace_all(html, " ");
    let step2 = re_style.replace_all(&step1, " ");
    let step3 = re_svg.replace_all(&step2, " ");
    // 去除 HTML 标签
    let re_tag = Regex::new(r"<[^>]+>").unwrap();
    let text = re_tag.replace_all(&step3, " ");
    // HTML 实体解码
    let text = text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    // 合并空白
    let re_ws = Regex::new(r"\s+").unwrap();
    re_ws.replace_all(&text, " ").trim().to_string()
}

/// 从 HTML 提取标题
fn extract_title(html: &str) -> String {
    let re = Regex::new(r"(?i)<title>([^<]*)</title>").unwrap();
    re.captures(html)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default()
}

/// 从 HTML 提取正文（mdbook-content 区域）
fn extract_body(html: &str) -> String {
    // 优先匹配 id="mdbook-content"，再匹配 class="content"，最后匹配 <main>
    let re1 = Regex::new(r#"(?is)<div[^>]*id="mdbook-content"[^>]*>(.*?)</div>"#).unwrap();
    if let Some(caps) = re1.captures(html) {
        return caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    let re2 = Regex::new(r#"(?is)<div[^>]*class="content"[^>]*>(.*?)</div>"#).unwrap();
    if let Some(caps) = re2.captures(html) {
        return caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    let re3 = Regex::new(r"(?is)<main[^>]*>(.*?)</main>").unwrap();
    if let Some(caps) = re3.captures(html) {
        return caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    String::new()
}

/// 递归扫描 HTML 文件
fn scan_html_files(dir: &Path, base_url: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return files;
    }
    for entry in fs::read_dir(dir).unwrap_or_else(|_| panic!("无法读取目录: {:?}", dir)) {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let rel_path = if base_url.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", base_url, name)
        };

        if path.is_dir() {
            files.extend(scan_html_files(&path, &rel_path));
        } else if name.ends_with(".html")
            && name != "print.html"
            && name != "404.html"
        {
            files.push((path.to_string_lossy().to_string(), rel_path));
        }
    }
    files
}

/// 运行 build-search
pub fn run(html_dir: &str) -> anyhow::Result<()> {
    let html_path = Path::new(html_dir);
    if !html_path.is_dir() {
        anyhow::bail!("目录不存在: {}", html_dir);
    }

    // 1. 扫描 HTML 文件
    eprintln!("🔍 扫描 HTML 文件...");
    let html_files = scan_html_files(html_path, "");
    eprintln!("   找到 {} 个 HTML 文件", html_files.len());

    // 2. 提取文档
    let mut docs: Vec<SearchDoc> = Vec::new();
    for (path, url) in &html_files {
        let html = fs::read_to_string(path)?;
        let title = extract_title(&html);
        let body_html = extract_body(&html);
        let body = extract_text(&body_html);
        docs.push(SearchDoc {
            id: docs.len(),
            url: url.clone(),
            title: title.clone(),
            body: body.clone(),
        });
    }
    eprintln!("   提取 {} 个文档", docs.len());

    // 3. 构建倒排索引（bigram 分词）
    eprintln!("🔧 构建搜索索引（中文 bigram 分词）...");
    let mut index = SearchIndex {
        documents: docs,
        tokens: HashMap::new(),
    };

    for doc in &index.documents {
        let text = format!("{} {}", doc.title, doc.body);
        let tokens = cjk_bigram_tokenize(&text.to_lowercase());
        for token in tokens {
            index.tokens.entry(token).or_default().push(doc.id);
        }
    }

    // 4. 输出搜索索引（JS 文件，用 script 标签加载，避免 file:// CORS 限制）
    let json = serde_json::to_string(&index)?;

    // 删除旧格式的 searchindex.json（如果存在）
    let old_path = html_path.join("searchindex.json");
    if old_path.exists() {
        fs::remove_file(&old_path)?;
    }

    let js_path = html_path.join("searchindex.js");
    let js_content = format!("window.searchData = {};", json);
    fs::write(&js_path, &js_content)?;
    eprintln!("   输出 searchindex.js ({:.0} KB)", js_content.len() as f64 / 1024.0);

    // 5. 输出 searcher.js
    let searcher_path = html_path.join("searcher.js");
    fs::write(&searcher_path, SEARCHER_JS)?;
    eprintln!("   输出 searcher.js（中文 bigram 搜索版）");

    // 6. 删除不再需要的搜索库文件
    for filename in &["elasticlunr.min.js", "minisearch.umd.js"] {
        let path = html_path.join(filename);
        if path.exists() {
            fs::remove_file(&path)?;
            eprintln!("   删除 {}", filename);
        }
    }

    // 7. 替换所有 HTML 中的旧搜索库引用（删除整个 script 标签）
    let mut replaced = 0;
    let re_old_search = Regex::new(r#"(?i)<script[^>]*?(?:elasticlunr\.min\.js|minisearch\.umd\.js)[^>]*>\s*</script>"#).unwrap();
    for (path, _url) in &html_files {
        let html = fs::read_to_string(path)?;
        let new_html = re_old_search.replace_all(&html, "");
        if new_html != html {
            fs::write(path, new_html.as_ref())?;
            replaced += 1;
        }
    }
    // 也处理根目录的 HTML
    for entry in fs::read_dir(html_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |e| e == "html") {
            let html = fs::read_to_string(&path)?;
            let new_html = re_old_search.replace_all(&html, "");
            if new_html != html {
                fs::write(&path, new_html.as_ref())?;
                replaced += 1;
            }
        }
    }
    eprintln!("   已清理 {} 个 HTML 文件中的旧搜索库引用", replaced);

    eprintln!("✅ 中文搜索索引构建完成！");
    Ok(())
}

/// 嵌入的 searcher.js — 前端中文 bigram 搜索实现
const SEARCHER_JS: &str = r###"/**
 * mdbook 中文搜索 — 基于 bigram 分词的轻量级搜索
 *
 * 替换 mdbook 默认的 elasticlunr 搜索，支持中文全文搜索。
 * 需配合 build-search 生成的 searchindex.json 使用。
 * 零外部依赖（除 Mark.js 用于高亮）。
 */
(function () {
    'use strict';

    // ===== DOM 引用 =====
    var searchWrap = document.getElementById('mdbook-search-wrapper');
    var searchbarOuter = document.getElementById('mdbook-searchbar-outer');
    var searchbar = document.getElementById('mdbook-searchbar');
    var searchResults = document.getElementById('mdbook-searchresults');
    var searchResultsOuter = document.getElementById('mdbook-searchresults-outer');
    var searchResultsHeader = document.getElementById('mdbook-searchresults-header');
    var searchIcon = document.getElementById('mdbook-search-toggle');
    var content = document.getElementById('mdbook-content');

    if (!searchWrap || !searchbar || !searchResults) return;

    // ===== 状态 =====
    var searchData = null;      // { documents, tokens }
    var currentSearchTerm = '';
    var limitResults = 30;

    // 高亮标记器
    var marker = null;
    if (typeof Mark !== 'undefined' && content) {
        marker = new Mark(content);
    }

    // ===== CJK bigram 分词器（与 Rust 端一致） =====
    function isCJK(ch) {
        var code = ch.charCodeAt(0);
        return (code >= 0x4E00 && code <= 0x9FFF)
            || (code >= 0x3400 && code <= 0x4DBF)
            || (code >= 0xF900 && code <= 0xFAFF);
    }

    function tokenize(text) {
        if (!text || typeof text !== 'string') return [];
        var tokens = [];
        var cjkBuf = [];
        var latinBuf = '';

        for (var i = 0; i < text.length; i++) {
            var ch = text[i].toLowerCase();
            if (isCJK(ch)) {
                if (latinBuf) { tokens.push(latinBuf); latinBuf = ''; }
                cjkBuf.push(ch);
            } else if (/[a-zA-Z0-9]/.test(ch)) {
                if (cjkBuf.length) {
                    tokens = tokens.concat(cjkBigramTokens(cjkBuf));
                    cjkBuf = [];
                }
                latinBuf += ch;
            } else {
                if (cjkBuf.length) {
                    tokens = tokens.concat(cjkBigramTokens(cjkBuf));
                    cjkBuf = [];
                }
                if (latinBuf) { tokens.push(latinBuf); latinBuf = ''; }
            }
        }
        if (cjkBuf.length) tokens = tokens.concat(cjkBigramTokens(cjkBuf));
        if (latinBuf) tokens.push(latinBuf);

        // 去重
        tokens.sort();
        return tokens.filter(function (t, i) { return i === 0 || t !== tokens[i - 1]; });
    }

    function cjkBigramTokens(buf) {
        var result = [];
        for (var i = 0; i < buf.length; i++) {
            result.push(buf[i]); // unigram
        }
        for (var i = 0; i < buf.length - 1; i++) {
            result.push(buf[i] + buf[i + 1]); // bigram
        }
        return result;
    }

    // ===== 工具函数 =====
    function hasFocus() {
        return searchbar === document.activeElement;
    }

    function removeChildren(elem) {
        while (elem.firstChild) elem.removeChild(elem.firstChild);
    }

    function escapeHtml(str) {
        return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    }

    function escapeRegex(str) {
        return str.replace(/[-/\\^$*+?.()|[\]{}]/g, '\\$&');
    }

    // ===== Teaser（摘要）生成 =====
    function makeTeaser(body, searchTerms) {
        if (!body || !searchTerms || searchTerms.length === 0) return '';

        var lowerBody = body.toLowerCase();
        var matchIndex = -1;
        for (var i = 0; i < searchTerms.length; i++) {
            var idx = lowerBody.indexOf(searchTerms[i].toLowerCase());
            if (idx >= 0 && (matchIndex < 0 || idx < matchIndex)) {
                matchIndex = idx;
            }
        }
        if (matchIndex < 0) return body.slice(0, 200);

        var start = Math.max(0, matchIndex - 60);
        var end = Math.min(body.length, matchIndex + 180);
        var teaser = (start > 0 ? '…' : '') + body.slice(start, end) + (end < body.length ? '…' : '');

        for (var j = 0; j < searchTerms.length; j++) {
            if (!searchTerms[j]) continue;
            var re = new RegExp('(' + escapeRegex(searchTerms[j]) + ')', 'gi');
            teaser = teaser.replace(re, '<mark>$1</mark>');
        }
        return teaser;
    }

    // ===== 搜索结果 =====
    function displayResults(results) {
        removeChildren(searchResults);

        if (!results || results.length === 0) {
            searchResultsHeader.innerText = '未找到结果';
            searchResultsOuter.classList.remove('hidden');
            return;
        }

        searchResultsHeader.innerText = '搜索结果:';
        var limit = Math.min(limitResults, results.length);

        for (var i = 0; i < limit; i++) {
            var r = results[i];
            var doc = searchData.documents[r.id];
            if (!doc) continue;

            var li = document.createElement('li');
            li.className = 'searchresult';

            var a = document.createElement('a');
            var targetUrl = (typeof path_to_root !== 'undefined' ? path_to_root : '') + doc.url;
            // 对 URL 中的中文等非 ASCII 字符进行编码，确保文件协议下可跳转
            a.href = encodeURI(targetUrl);
            a.className = 'searchresult-link';
            // 确保在新页面中正确打开
            a.target = '_self';

            var titleSpan = document.createElement('span');
            titleSpan.className = 'searchresult-title';
            titleSpan.textContent = doc.title;
            a.appendChild(titleSpan);

            var teaser = makeTeaser(doc.body, r.terms);
            if (teaser) {
                var teaserSpan = document.createElement('span');
                teaserSpan.className = 'searchresult-teaser';
                teaserSpan.innerHTML = teaser;
                a.appendChild(teaserSpan);
            }

            li.appendChild(a);

            // 确保点击跳转正常工作
            li.addEventListener('click', function (e) {
                var target = e.target.closest('.searchresult-link');
                if (target) {
                    e.preventDefault();
                    window.location.href = target.href;
                }
            });

            searchResults.appendChild(li);
        }
        searchResultsOuter.classList.remove('hidden');
    }

    // ===== 执行搜索 =====
    function doSearch(searchTerm) {
        if (!searchData || !searchTerm) {
            searchResultsOuter.classList.add('hidden');
            return;
        }

        var searchTokens = tokenize(searchTerm);
        if (searchTokens.length === 0) {
            searchResultsOuter.classList.add('hidden');
            return;
        }

        // 在倒排索引中查找匹配文档
        var docScores = {};  // doc_id -> { score: number, terms: [string, ...] }

        for (var t = 0; t < searchTokens.length; t++) {
            var token = searchTokens[t];
            var matchedIds = searchData.tokens[token];
            if (!matchedIds) continue;

            for (var m = 0; m < matchedIds.length; m++) {
                var docId = matchedIds[m];
                if (!docScores[docId]) {
                    docScores[docId] = { score: 0, terms: [] };
                }
                docScores[docId].score++;
                if (docScores[docId].terms.indexOf(token) < 0) {
                    docScores[docId].terms.push(token);
                }
            }
        }

        // 标题匹配加权
        for (var docId in docScores) {
            var doc = searchData.documents[parseInt(docId)];
            if (!doc) continue;
            var titleLower = doc.title.toLowerCase();
            for (var t = 0; t < searchTokens.length; t++) {
                if (titleLower.indexOf(searchTokens[t].toLowerCase()) >= 0) {
                    docScores[docId].score += 2;
                }
            }
        }

        // 按得分排序
        var sorted = Object.keys(docScores)
            .map(function (id) { return { id: parseInt(id), score: docScores[id].score, terms: docScores[id].terms }; })
            .sort(function (a, b) { return b.score - a.score; });

        displayResults(sorted);
    }

    // ===== 搜索输入处理 =====
    var searchTimeout = null;

    function onSearchInput() {
        var searchTerm = searchbar.value.trim();
        currentSearchTerm = searchTerm;

        if (searchTimeout) clearTimeout(searchTimeout);

        if (!searchTerm) {
            searchResultsOuter.classList.add('hidden');
            return;
        }

        searchTimeout = setTimeout(function () {
            doSearch(searchTerm);
        }, 150);

        updateURLParam(searchTerm);
    }

    function updateURLParam(searchTerm) {
        if (history.replaceState) {
            var url = new URL(window.location);
            if (searchTerm) url.searchParams.set('search', searchTerm);
            else url.searchParams.delete('search');
            history.replaceState(null, '', url);
        }
    }

    // ===== 搜索框显示/隐藏 =====
    function showSearchbar() {
        searchWrap.classList.remove('hidden');
        searchbarOuter.classList.remove('hidden');
        searchbar.focus();
        searchbar.select();
        searchbar.dispatchEvent(new Event('input'));
    }

    function hideSearchbar() {
        searchWrap.classList.add('hidden');
        searchbarOuter.classList.add('hidden');
        searchResultsOuter.classList.add('hidden');
        searchbar.blur();
    }

    function toggleSearchbar() {
        if (searchWrap.classList.contains('hidden')) showSearchbar();
        else hideSearchbar();
    }

    // ===== 加载搜索索引（使用 script 标签避免 file:// CORS 限制） =====
    function loadIndex(callback) {
        var script = document.createElement('script');
        script.src = (typeof path_to_root !== 'undefined' ? path_to_root : '') + 'searchindex.js';
        script.onload = function () {
            if (window.searchData) {
                callback(window.searchData);
            } else {
                console.error('searchindex.js 加载完成但数据为空');
            }
        };
        script.onerror = function () {
            console.error('无法加载 searchindex.js（请先运行 mdbook-plugins build-search）');
        };
        document.head.appendChild(script);
    }

    // ===== 初始化 =====
    function init() {
        loadIndex(function (data) {
            searchData = data;

            // 从 URL 参数恢复搜索
            var urlParams = new URLSearchParams(window.location.search);
            var searchTerm = urlParams.get('search');
            if (searchTerm) {
                searchbar.value = searchTerm;
                showSearchbar();
                doSearch(searchTerm);
            }
        });
    }

    // ===== 事件绑定 =====
    function initEvents() {
        searchbar.addEventListener('input', onSearchInput);

        if (searchIcon) {
            searchIcon.addEventListener('click', toggleSearchbar);
        }

        document.addEventListener('keydown', function (e) {
            if ((e.key === 's' || e.key === 'S' || e.key === '/')
                && !hasFocus() && !e.ctrlKey && !e.metaKey) {
                e.preventDefault();
                showSearchbar();
                return;
            }
            if (e.key === 'Escape' && hasFocus()) {
                hideSearchbar();
                return;
            }
            if (e.key === 'Enter' && hasFocus()) {
                var firstResult = searchResults.querySelector('.searchresult-link');
                if (firstResult) window.location.href = firstResult.href;
            }
        });

        document.addEventListener('click', function (e) {
            if (!searchWrap.contains(e.target)
                && !searchIcon.contains(e.target)
                && !searchWrap.classList.contains('hidden')) {
                hideSearchbar();
            }
        });
    }

    // ===== 启动 =====
    if (document.readyState === 'complete' || document.readyState === 'interactive') {
        init();
        initEvents();
    } else {
        document.addEventListener('DOMContentLoaded', function () {
            init();
            initEvents();
        });
    }
})();
"###;
