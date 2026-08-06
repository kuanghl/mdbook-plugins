/**
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
