// 语言切换按钮：点击 globe 弹出语言选择框，选完自动收回
(function() {
    var LANGUAGES = [
        { id: 'english', name: 'English' },
        { id: 'chinese_simplified', name: '简体中文' },
    ];
    var popup = null;

    // 忽略翻译的class id
    translate.ignore.class.push("icon-button");
    translate.ignore.class.push("theme-popup");
    translate.ignore.class.push('MathJax');
    translate.ignore.class.push('katex-src');
    translate.ignore.class.push('katex-display');
    translate.ignore.class.push('chapter-item');
    translate.ignore.class.push('mermaid');
    translate.ignore.tag.push('text');

    // 不显示默认的select语言选择框
    translate.selectLanguageTag.show = false;

    // 执行翻译
    translate.execute();
    translate.request.listener.start();

    function closePopup() {
        if (popup) {
            popup.remove();
            popup = null;
        }
    }

    function createPopup(btn) {
        closePopup();

        popup = document.createElement('div');
        popup.id = 'translate-popup';
        popup.style.cssText =
            'position:fixed;z-index:9999;background:var(--theme-popup-bg,#fff);' +
            'border:1px solid var(--theme-popup-border,#ccc);border-radius:6px;' +
            'box-shadow:0 2px 12px rgba(0,0,0,0.15);padding:4px 0;min-width:140px;';

        LANGUAGES.forEach(function(lang) {
            var item = document.createElement('div');
            item.textContent = lang.name;
            item.style.cssText =
                'padding:8px 16px;cursor:pointer;font-size:14px;' +
                'color:var(--fg,#333);white-space:nowrap;';
            item.onmouseenter = function() {
                item.style.background = 'var(--theme-hover,#f0f0f0)';
            };
            item.onmouseleave = function() {
                item.style.background = 'transparent';
            };
            item.onclick = function() {
                translate.changeLanguage(lang.id);
                closePopup();
            };
            popup.appendChild(item);
        });

        // 定位到按钮下方
        document.body.appendChild(popup);
        var rect = btn.getBoundingClientRect();
        var top = rect.bottom + 4;
        var left = rect.left;
        // 确保不超出右边界
        if (left + 160 > window.innerWidth) {
            left = window.innerWidth - 160;
        }
        popup.style.top = top + 'px';
        popup.style.left = left + 'px';

        // 点击其他地方关闭
        setTimeout(function() {
            document.addEventListener('click', closePopup, { once: true });
        }, 10);
    }

    function bindClick() {
        var btn = document.getElementById('translate');
        if (!btn) {
            setTimeout(bindClick, 200);
            return;
        }
        btn.addEventListener('click', function(e) {
            e.stopPropagation();
            if (popup) {
                closePopup();
            } else {
                createPopup(btn);
            }
        });
    }

    setTimeout(bindClick, 500);
})();
