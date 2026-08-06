// ============================================================
// mdbook-plugins 辅助脚本
// ============================================================

// ==================== 1. Catppuccin 主题高亮修复 ====================
(function() {
    var DARK_THEMES = ['mocha', 'macchiato', 'frappe', 'coal', 'navy', 'ayu'];
    var LIGHT_THEMES = ['latte', 'light', 'rust'];

    function fixHighlight(theme) {
        var ayu = document.getElementById('mdbook-ayu-highlight-css');
        var tomorrow = document.getElementById('mdbook-tomorrow-night-css');
        var highlight = document.getElementById('mdbook-highlight-css');
        if (!ayu || !tomorrow || !highlight) return;
        if (DARK_THEMES.indexOf(theme) >= 0) {
            ayu.disabled = (theme !== 'ayu');
            tomorrow.disabled = false;
            highlight.disabled = true;
        } else {
            ayu.disabled = true;
            tomorrow.disabled = true;
            highlight.disabled = false;
        }
    }

    var origSetItem = localStorage.setItem;
    localStorage.setItem = function(key, value) {
        origSetItem.call(localStorage, key, value);
        if (key === 'mdbook-theme') fixHighlight(value);
    };

    document.addEventListener('DOMContentLoaded', function() {
        var theme = localStorage.getItem('mdbook-theme');
        if (theme) fixHighlight(theme);
    });

    var observer = new MutationObserver(function(mutations) {
        mutations.forEach(function(m) {
            if (m.attributeName === 'class') {
                var html = document.documentElement;
                for (var i = 0; i < DARK_THEMES.length; i++) {
                    if (html.classList.contains(DARK_THEMES[i])) {
                        fixHighlight(DARK_THEMES[i]);
                        return;
                    }
                }
                for (var i = 0; i < LIGHT_THEMES.length; i++) {
                    if (html.classList.contains(LIGHT_THEMES[i])) {
                        fixHighlight(LIGHT_THEMES[i]);
                        return;
                    }
                }
            }
        });
    });
    observer.observe(document.documentElement, { attributes: true });
})();

// ==================== 2. 捐助按钮 → GitHub Sponsors ====================
(function() {
    function fixSponsorButton() {
        var links = document.querySelectorAll('a[title="Sponsor"], a[aria-label="Sponsor"]');
        for (var i = 0; i < links.length; i++) {
            links[i].href = 'https://github.com/sponsors/kuanghl';
            links[i].target = '_blank';
            links[i].rel = 'noopener noreferrer';
        }
    }
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', fixSponsorButton);
    } else {
        fixSponsorButton();
    }
})();

// ==================== 3. 打印按钮修复 ====================
// 打印按钮默认 <a href="../print.html"> 跳转到 print.html，
// 但 print.html 的自动打印依赖 MathJax，CDN 加载慢或失败时打印永不触发。
// 改为直接在当前页触发 window.print() —— 最可靠的方式。
(function() {
    function fixPrintButton() {
        var links = document.querySelectorAll('a[title="Print this book"], a[aria-label="Print this book"]');
        for (var i = 0; i < links.length; i++) {
            links[i].addEventListener('click', function(e) {
                e.preventDefault();
                window.print();
            });
        }
    }
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', fixPrintButton);
    } else {
        fixPrintButton();
    }
})();

// ==================== 4. print.html 自动打印（兜底） ====================
// 如果用户直接打开 print.html（而非通过按钮点击），仍然自动触发打印。
(function() {
    if (!window.location.pathname.match(/print\.html/)) return;

    var PRINT_DONE = false;
    function doPrint() {
        if (PRINT_DONE) return;
        PRINT_DONE = true;

        // 如果 MathJax 可用，等待它完成后再打印
        if (window.MathJax && MathJax.Hub) {
            MathJax.Hub.Register.StartupHook('End', function() {
                setTimeout(function() { window.print(); }, 200);
            });
            // 兜底：5 秒后强制打印
            setTimeout(function() {
                if (!PRINT_DONE) { PRINT_DONE = true; window.print(); }
            }, 5000);
        } else {
            // MathJax 不可用，直接打印
            setTimeout(function() { window.print(); }, 1000);
        }
    }

    if (document.readyState === 'complete') {
        setTimeout(doPrint, 500);
    } else {
        window.addEventListener('load', function() { setTimeout(doPrint, 500); });
    }
})();

// ==================== 5. 浏览器打印页眉页脚 ====================
// 模拟 [output.pdf] 的 header-template / footer-template，
// 让 Ctrl+P 也能显示自定义页眉页脚。
// 默认隐藏，打印时固定定位。
(function() {
    var HEADER_HTML =
        '<div style="display:flex;justify-content:space-between;align-items:center;padding:6px 20px;font-size:9px;font-family:sans-serif;color:#555;">' +
            '<span class="print-date"></span>' +
            '<span class="print-title" style="font-weight:600;color:#333;">mdbook-demo</span>' +
            '<span>kuanghl</span>' +
        '</div>' +
        '<hr style="border:none;border-top:1px solid #ddd;margin:0 15px;">';

    var FOOTER_HTML =
        '<hr style="border:none;border-bottom:1px solid #ddd;margin:0 15px;">' +
        '<div style="display:flex;justify-content:space-between;align-items:center;padding:6px 20px;font-size:9px;font-family:sans-serif;color:#555;">' +
            '<span>版本号</span>' +
            '<span>页码</span>' +
            '<span>BSN</span>' +
        '</div>';

    function addPrintElements() {
        if (document.getElementById('mdbook-print-header')) return;
        var header = document.createElement('div');
        header.id = 'mdbook-print-header';
        header.innerHTML = HEADER_HTML;
        document.body.appendChild(header);
        var footer = document.createElement('div');
        footer.id = 'mdbook-print-footer';
        footer.innerHTML = FOOTER_HTML;
        document.body.appendChild(footer);
        var now = new Date();
        var dateStr = now.getFullYear() + '-' +
            String(now.getMonth() + 1).padStart(2, '0') + '-' +
            String(now.getDate()).padStart(2, '0');
        var dateEl = header.querySelector('.print-date');
        if (dateEl) dateEl.textContent = dateStr;
        var titleEl = header.querySelector('.print-title');
        if (titleEl) {
            titleEl.textContent = document.title || 'mdbook-demo';
        }
    }

    function addPrintStyles() {
        if (document.getElementById('mdbook-print-style')) return;
        var style = document.createElement('style');
        style.id = 'mdbook-print-style';
        style.textContent =
            '#mdbook-print-header,#mdbook-print-footer{display:none}' +
            '@media print{' +
                '#mdbook-print-header{position:fixed;top:0;left:0;right:0;z-index:999;background:#fff;display:block!important}' +
                '#mdbook-print-footer{position:fixed;bottom:0;left:0;right:0;z-index:999;background:#fff;display:block!important}' +
            '}';
        document.head.appendChild(style);
    }

    function init() {
        addPrintStyles();
        addPrintElements();
    }
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
