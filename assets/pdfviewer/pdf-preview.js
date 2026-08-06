/**
 * mdbook-pdf-preview — 嵌入式 PDF 预览运行时（v6: 纯 PDFObject）
 *
 * 由 mdbook-plugins pdf-preview 预处理器配合使用，通过 book.toml 的
 * additional-js 加载（须在 pdfobject.min.js 之后）。
 *
 * 实现：滚动到视口后，用 PDFObject（~5KB）把 PDF 内嵌到容器，
 * 由浏览器原生 PDF viewer 渲染（Chrome/Edge/Firefox/Safari 均支持）。
 * 浏览器不支持原生 PDF 时显示提示与下载链接，不再依赖 pdf.js。
 */
(function () {
  'use strict';

  var VERSION = 'v6 (PDFObject)';
  if (window.console && console.debug) {
    console.debug('[mdbook-pdf-preview]', VERSION);
  }

  // ================================================================
  // 主题配置（跟随 mdbook 主题，通过 CSS 变量作用到容器）
  // ================================================================
  var THEME_MAP = {
    light:     { bg: '#ffffff', canvas: '#e8e8e8', text: '#333', muted: '#888' },
    rust:      { bg: '#ffffff', canvas: '#e8e8e8', text: '#333', muted: '#888' },
    coal:      { bg: '#1d1f21', canvas: '#2a2c2e', text: '#ccc', muted: '#888' },
    navy:      { bg: '#161923', canvas: '#1e2433', text: '#bcbdd0', muted: '#6b7089' },
    ayu:       { bg: '#0f1419', canvas: '#1a1f26', text: '#bfc7d5', muted: '#6c7886' },
    latte:     { bg: '#fff',    canvas: '#e6e9ef', text: '#4c4f69', muted: '#9ca0b0' },
    frappe:    { bg: '#303446', canvas: '#414559', text: '#c6d0f5', muted: '#949cbb' },
    macchiato: { bg: '#24273a', canvas: '#363a4f', text: '#cad3f5', muted: '#a5adcb' },
    mocha:     { bg: '#1e1e2e', canvas: '#313244', text: '#cdd6f4', muted: '#a6adc8' },
  };
  var DEFAULT_THEME = THEME_MAP.light;

  function getThemeColors() {
    var html = document.documentElement;
    for (var cls in THEME_MAP) {
      if (THEME_MAP.hasOwnProperty(cls) && html.classList.contains(cls)) {
        return THEME_MAP[cls];
      }
    }
    return DEFAULT_THEME;
  }

  function applyTheme() {
    var c = getThemeColors();
    var root = document.documentElement;
    var border = c.canvas === '#e8e8e8' ? '#d0d0d0' : '#444';
    root.style.setProperty('--ppv-border', border);
    root.style.setProperty('--ppv-bg', c.bg);
    root.style.setProperty('--ppv-text', c.text);
    root.style.setProperty('--ppv-muted', c.muted);
    root.style.setProperty('--ppv-title', c.text);
    root.style.setProperty('--ppv-hover-bg', c.canvas);
    root.style.setProperty('--ppv-canvas-bg', c.canvas);
    return c;
  }

  // ================================================================
  // 工具
  // ================================================================

  /** 将相对路径解析为绝对 URL（相对当前页面） */
  function resolveURL(url) {
    var a = document.createElement('a');
    a.href = url;
    return a.href;
  }

  function getFilename(url) {
    try {
      return decodeURIComponent(url.replace(/[?#].*$/, '').split('/').pop() || url);
    } catch (e) {
      return url;
    }
  }

  function escapeHtml(s) {
    var d = document.createElement('div');
    d.appendChild(document.createTextNode(s));
    return d.innerHTML;
  }

  function isFileProtocol() {
    return window.location.protocol === 'file:';
  }

  /** 浏览器不支持原生 PDF 时：提示 + 下载链接 */
  function renderUnsupported(container, absUrl) {
    var filename = getFilename(absUrl);
    container.innerHTML =
      '<div class="ppv-error">当前浏览器不支持内嵌 PDF 预览，' +
      '请 <a href="' + escapeHtml(absUrl) + '" target="_blank" rel="noopener">下载 ' +
      escapeHtml(filename) + '</a> 查看。</div>';
  }

  /** file:// 协议：直接用 <embed> 原生渲染（Chrome 允许 embed 加载 file:// PDF） */
  function renderWithEmbed(container, pdfUrl) {
    var c = applyTheme();
    var absUrl = resolveURL(pdfUrl);
    var filename = getFilename(pdfUrl);
    var borderColor = c.canvas === '#e8e8e8' ? '#d0d0d0' : '#444';
    container.innerHTML =
      '<div class="ppv-embed-bar" style="background:' + c.canvas + ';color:' + c.text + ';border-bottom:1px solid ' + borderColor + ';">' +
        '<span class="ppv-embed-icon">' +
          '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 512 512" style="fill:' + c.text + '">' +
            '<path d="M64 464l48 0 0 48-48 0c-35.3 0-64-28.7-64-64L0 64C0 28.7 28.7 0 64 0L229.5 0c17 0 33.3 6.7 45.3 18.7l90.5 90.5c12 12 18.7 28.3 18.7 45.3L384 304l-48 0 0-144-80 0c-17.7 0-32-14.3-32-32l0-80L64 48c-8.8 0-16 7.2-16 16l0 384c0 8.8 7.2 16 16 16zM176 352l32 0c30.9 0 56 25.1 56 56s-25.1 56-56 56l-16 0 0 32c0 8.8-7.2 16-16 16s-16-7.2-16-16l0-128c0-8.8 7.2-16 16-16zm32 80c13.3 0 24-10.7 24-24s-10.7-24-24-24l-16 0 0 48 16 0zm96-80l32 0c26.5 0 48 21.5 48 48l0 64c0 26.5-21.5 48-48 48l-32 0c-8.8 0-16-7.2-16-16l0-128c0-8.8 7.2-16 16-16zm32 128c8.8 0 16-7.2 16-16l0-64c0-8.8-7.2-16-16-16l-16 0 0 96 16 0zm80-112c0-8.8 7.2-16 16-16l48 0c8.8 0 16 7.2 16 16s-7.2 16-16 16l-32 0 0 32 32 0c8.8 0 16 7.2 16 16s-7.2 16-16 16l-32 0 0 48c0 8.8-7.2 16-16 16s-16-7.2-16-16l0-128z"/>' +
          '</svg>' +
          '<span>' + escapeHtml(filename) + '</span>' +
        '</span>' +
      '</div>' +
      '<embed src="' + absUrl + '" type="application/pdf" style="width:100%;height:80vh;border:none;display:block;">';
  }

  /** HTTP 环境：PDFObject 内嵌浏览器原生 PDF viewer */
  function renderWithPDFObject(container, pdfUrl) {
    if (typeof PDFObject === 'undefined') {
      container.innerHTML = '<div class="ppv-error">❌ pdfobject.min.js 未加载</div>';
      return;
    }
    var absUrl = resolveURL(pdfUrl);
    // cache-buster query：规避浏览器扩展/安全功能对"首次 .pdf 请求"的拦截
    // （症状：ERR_BLOCKED_BY_CLIENT / 弹窗下载）。静态服务器忽略 query。
    var sep = absUrl.indexOf('?') === -1 ? '?' : '&';
    var url = absUrl + sep + 't=' + Date.now();

    var ok = PDFObject.embed(url, container, {
      height: '100vh',
      pdfOpenParams: { view: 'FitH' },
    });
    if (!ok) {
      renderUnsupported(container, absUrl);
    }
  }

  // ================================================================
  // 初始化
  // ================================================================

  function init() {
    var containers = document.querySelectorAll('.pdfviewer-container');
    if (containers.length === 0) return;
    applyTheme();

    if (isFileProtocol()) {
      // file://：Chrome 允许 <embed> 加载 file:// PDF，直接渲染
      for (var i = 0; i < containers.length; i++) {
        (function (c) {
          if (c.getAttribute('data-ppv-ready')) return;
          c.setAttribute('data-ppv-ready', 'true');
          var pdfUrl = c.getAttribute('data-pdf-src');
          if (!pdfUrl) return;
          renderWithEmbed(c, pdfUrl);
        })(containers[i]);
      }
      return;
    }

    // HTTP：IntersectionObserver 视口自加载
    function startRender(c) {
      if (c.getAttribute('data-ppv-ready')) return;
      c.setAttribute('data-ppv-ready', 'true');
      var pdfUrl = c.getAttribute('data-pdf-src');
      if (!pdfUrl) return;
      renderWithPDFObject(c, pdfUrl);
    }

    if ('IntersectionObserver' in window) {
      var observer = new IntersectionObserver(function (entries) {
        for (var i = 0; i < entries.length; i++) {
          if (entries[i].isIntersecting) {
            observer.unobserve(entries[i].target);
            startRender(entries[i].target);
          }
        }
      }, { rootMargin: '200px' }); // 提前 200px 预加载

      for (var i = 0; i < containers.length; i++) {
        observer.observe(containers[i]);
      }
    } else {
      for (var i = 0; i < containers.length; i++) {
        startRender(containers[i]);
      }
    }
  }

  function observeTheme() {
    var observer = new MutationObserver(function () {
      applyTheme();
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function () { init(); observeTheme(); });
  } else {
    init(); observeTheme();
  }
})();
