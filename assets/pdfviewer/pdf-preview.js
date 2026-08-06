/**
 * mdbook-pdf-preview — 嵌入式 PDF 预览运行时
 *
 * 由 mdbook-plugins pre-pdfviewer 预处理器配合使用。
 * 通过 book.toml 的 additional-js 加载。
 *
 * 双渲染模式（由 preprocessor 生成容器的 data-preview-mode 决定）：
 *   - embed（默认）：<iframe> 内嵌，浏览器原生 PDF viewer 渲染，零 JS 依赖。
 *     适合 mdbook serve / 部署的 http 环境。
 *   - pdfjs：pdf.js Canvas 渲染，支持翻页/缩放/关闭，主题跟随。
 *     通过 [preprocessor.pdf-preview] mode = "pdfjs" 启用。
 *
 * file:// 直接打开构建产物时，浏览器把每个 file: 当作唯一安全源，
 * 会拦截 iframe 与 fetch 加载 PDF —— 两种模式都无法工作，给出明确提示与解法。
 */
(function () {
  'use strict';

  var VERSION = 'v5 (dual-mode: embed|pdfjs)';
  if (window.console && console.debug) {
    console.debug('[mdbook-pdf-preview]', VERSION);
  }

  // ================================================================
  // 配置
  // ================================================================
  var CONFIG = {
    // CDN 路径（本地加载失败时回退）
    cdnJsPath: 'https://cdnjs.cloudflare.com/ajax/libs/pdf.js/6.1.200/pdf.min.mjs',
    cdnWorkerPath: 'https://cdnjs.cloudflare.com/ajax/libs/pdf.js/6.1.200/pdf.worker.min.mjs',
    // 本地 fallback 路径（通过脚本位置自动检测）
    defaultZoom: 1.0,
  };
  // 本地 fallback 路径
  var _scripts = document.querySelectorAll('script[src*="pdf-preview"]');
  for (var _i = 0; _i < _scripts.length; _i++) {
    var src = _scripts[_i].getAttribute('src');
    if (src && src.indexOf('pdf-preview') !== -1) {
      var a = document.createElement('a');
      a.href = src;
      var absSrc = a.href;
      var baseUrl = absSrc.substring(0, absSrc.lastIndexOf('/'));
      CONFIG.localJsPath = baseUrl + '/build/pdf.mjs';
      CONFIG.localWorkerPath = baseUrl + '/build/pdf.worker.mjs';
      // pdf.js 完整 viewer（viewer.html 同源加载，无需 CORS；UI 接近浏览器原生）
      CONFIG.viewerPath = baseUrl + '/web/viewer.html';
      break;
    }
  }

  // ================================================================
  // 主题配置
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

  // ================================================================
  // 状态
  // ================================================================
  var pdfjsReady = false;
  var instances = [];

  // ================================================================
  // 工具
  // ================================================================

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
    root.style.setProperty('--ppv-toolbar-bg', c.canvas);
    root.style.setProperty('--ppv-btn-bg', c.bg);
    root.style.setProperty('--ppv-btn-hover', c.canvas);
    root.style.setProperty('--ppv-btn-active', c.canvas);
    root.style.setProperty('--ppv-canvas-bg', c.canvas);
    return c;
  }

  /** 将相对路径解析为绝对 URL（相对当前页面） */
  function resolveURL(url) {
    var a = document.createElement('a');
    a.href = url;
    return a.href;
  }

  function getFilename(url) {
    return decodeURIComponent((url.replace(/[?#].*$/, '').split('/').pop() || url));
  }

  function escapeHtml(s) {
    var d = document.createElement('div');
    d.appendChild(document.createTextNode(s));
    return d.innerHTML;
  }

  /** 检测是否为 file:// 协议 */
  function isFileProtocol() {
    return window.location.protocol === 'file:';
  }

  /** file:// 协议降级方案：使用浏览器原生 <embed> 渲染 PDF。
   *  原版行为——Chrome 对 embed 的 PDF viewer 允许加载 file:// PDF
   *  （iframe/fetch 才会被 file 唯一源策略拦截）。 */
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

  /** 尝试加载 pdf.js（本地 ESM 优先 → CDN ESM 回退） */
  function loadPDFJS(callback) {
    if (pdfjsReady) { callback(); return; }
    if (typeof pdfjsLib !== 'undefined' && pdfjsLib.GlobalWorkerOptions) {
      pdfjsReady = true;
      callback();
      return;
    }

    function showError() {
      // 只清空 pdfjs 模式的容器，避免误伤已渲染的 embed iframe
      var containers = document.querySelectorAll('.pdfviewer-container[data-preview-mode="pdfjs"]');
      for (var i = 0; i < containers.length; i++) {
        containers[i].innerHTML = '<div class="ppv-error">❌ PDF 库加载失败（本地与 CDN 均不可用）</div>';
      }
    }

    // 加载队列：本地 → CDN
    var sources = [
      { js: CONFIG.localJsPath, worker: CONFIG.localWorkerPath },
      { js: CONFIG.cdnJsPath,   worker: CONFIG.cdnWorkerPath },
    ];
    var sourceIndex = 0;

    function tryNext() {
      if (sourceIndex >= sources.length) {
        showError();
        return;
      }
      var src = sources[sourceIndex++];
      if (!src.js) { tryNext(); return; }

      import(src.js).then(function () {
        // pdf.min.mjs 内部已设置 globalThis.pdfjsLib = {…}
        if (src.worker && pdfjsLib.GlobalWorkerOptions) {
          pdfjsLib.GlobalWorkerOptions.workerSrc = src.worker;
        }
        pdfjsReady = true;
        callback();
      }).catch(function () {
        tryNext();
      });
    }

    tryNext();
  }

  // ================================================================
  // PDFViewer 实例（pdfjs 模式：pdf.js Canvas 渲染）
  // ================================================================

  function PDFViewer(container, pdfUrl) {
    this.container = container;
    this.pdfUrl = pdfUrl;
    this.absUrl = resolveURL(pdfUrl);   // 绝对 URL：避免相对路径在部分环境下解析失败
    this.pdf = null;
    this.pageNum = 1;
    this.canvas = null;
    this.ctx = null;
    this.pageCount = 0;
    this.zoom = CONFIG.defaultZoom;
    this._keyHandler = null;
  }

  PDFViewer.prototype.load = async function () {
    var self = this;
    self.container.innerHTML = '<div class="ppv-loading">加载 PDF 中</div>';
    try {
      // 三重兜底：绝对 URL → 原始相对路径 → 页面 URL，杜绝 getDocument 参数为空。
      // （正常情况下 absUrl 恒非空；兜底仅防御异常环境）
      var url = self.absUrl || self.pdfUrl || window.location.href;
      // disableRange/disableStream：完整 GET 加载，规避部分浏览器/扩展对
      // Range 流式请求返回 204（Unexpected server response (204)）的拦截。
      var task = pdfjsLib.getDocument({
        url: url,
        disableRange: true,
        disableStream: true,
        disableAutoFetch: false,
      });
      if (window.console && console.debug) {
        console.debug('[mdbook-pdf-preview] getDocument url =', url);
      }
      self.pdf = await task.promise;
      self.pageCount = self.pdf.numPages;
      self._buildUI();
      await self._renderPage();
    } catch (err) {
      // 204（无内容响应）：多为浏览器/扩展/缓存中间层拦截。
      // 追加 cache-buster query 重试一次（静态服务器忽略 query，但可绕过
      // 路径匹配型拦截与响应缓存）。
      var msg = err && err.message ? err.message : '';
      if (msg.indexOf('(204)') !== -1 && !self._retried) {
        self._retried = true;
        var sep = url.indexOf('?') === -1 ? '?' : '&';
        self.absUrl = url + sep + 't=' + Date.now();
        if (window.console && console.warn) {
          console.warn('[mdbook-pdf-preview] 收到 204，使用 cache-buster 重试:', self.absUrl);
        }
        return self.load();
      }
      self.container.innerHTML = '<div class="ppv-error">❌ ' + escapeHtml(msg || '加载失败') + '</div>';
    }
  };

  PDFViewer.prototype._buildUI = function () {
    var self = this;
    var c = applyTheme();
    self.container.innerHTML = '';

    // Toolbar
    var bar = document.createElement('div');
    bar.className = 'ppv-toolbar';
    bar.innerHTML =
      '<span class="ppv-page-info">第 <span class="ppv-page-num">1</span> / ' + self.pageCount + ' 页</span>' +
      '<div class="ppv-nav">' +
        '<button class="ppv-btn ppv-prev" title="上一页">‹</button>' +
        '<button class="ppv-btn ppv-next" title="下一页">›</button>' +
      '</div>' +
      '<div class="ppv-spacer"></div>' +
      '<button class="ppv-btn" title="放大">＋</button>' +
      '<button class="ppv-btn" title="缩小">－</button>' +
      '<button class="ppv-btn ppv-close" title="关闭预览">✕</button>';
    self.container.appendChild(bar);

    // Canvas wrapper
    var wrap = document.createElement('div');
    wrap.className = 'ppv-canvas-wrapper';
    self.canvas = document.createElement('canvas');
    self.ctx = self.canvas.getContext('2d');
    wrap.appendChild(self.canvas);
    self.container.appendChild(wrap);

    // Events
    var btns = bar.querySelectorAll('button');
    btns[0].onclick = function () { self.prevPage(); };
    btns[1].onclick = function () { self.nextPage(); };
    btns[2].onclick = function () { self.zoomIn(); };
    btns[3].onclick = function () { self.zoomOut(); };
    btns[4].onclick = function () { self.destroy(); };

    self._keyHandler = function (e) {
      if (e.key === 'Escape') { self.destroy(); return; }
      if (e.key === 'ArrowLeft') { e.preventDefault(); self.prevPage(); }
      if (e.key === 'ArrowRight') { e.preventDefault(); self.nextPage(); }
    };
    document.addEventListener('keydown', self._keyHandler);
  };

  PDFViewer.prototype._renderPage = async function () {
    if (!this.pdf) return;
    try {
      var page = await this.pdf.getPage(this.pageNum);
      var c = applyTheme();
      var wrap = this.container.querySelector('.ppv-canvas-wrapper');
      var maxW = wrap ? wrap.clientWidth : 800;
      var vp1 = page.getViewport({ scale: 1 });
      var scale = Math.min(this.zoom, maxW / vp1.width);
      var vp = page.getViewport({ scale: scale });

      // 设置 Canvas 缓冲区尺寸（渲染分辨率）
      this.canvas.width = vp.width;
      this.canvas.height = vp.height;
      // 设置 Canvas 显示尺寸（CSS 像素），用 aspect-ratio 保持宽高比
      // 避免 flex align-items:stretch 对高度的干扰
      var dispW = Math.min(vp.width, maxW);
      this.canvas.style.width = dispW + 'px';
      this.canvas.style.aspectRatio = vp1.width + ' / ' + vp1.height;
      this.canvas.style.background = '#fff';
      await page.render({ canvasContext: this.ctx, viewport: vp, background: '#fff' }).promise;

      var el = this.container.querySelector('.ppv-page-num');
      if (el) el.textContent = this.pageNum;
    } catch (e) { /* 忽略渲染中的竞态错误 */ }
  };

  PDFViewer.prototype.prevPage = function () { if (this.pageNum > 1) { this.pageNum--; this._renderPage(); } };
  PDFViewer.prototype.nextPage = function () { if (this.pageNum < this.pageCount) { this.pageNum++; this._renderPage(); } };
  PDFViewer.prototype.zoomIn = function () { this.zoom = Math.min(this.zoom * 1.25, 5); this._renderPage(); };
  PDFViewer.prototype.zoomOut = function () { this.zoom = Math.max(this.zoom / 1.25, 0.2); this._renderPage(); };

  PDFViewer.prototype.destroy = function () {
    if (this._keyHandler) document.removeEventListener('keydown', this._keyHandler);
    if (this.pdf) { this.pdf.destroy(); this.pdf = null; }
    instances = instances.filter(function (v) { return v !== this; }.bind(this));
    this.container.innerHTML = '';
  };

  // ================================================================
  // embed 模式：iframe 内嵌，浏览器原生 PDF viewer
  // ================================================================

  function loadNative(container) {
    var pdfUrl = container.getAttribute('data-pdf-src');
    if (!pdfUrl) return;
    var iframe = document.createElement('iframe');
    // cache-buster query：规避浏览器扩展/安全功能对"首次 .pdf 请求"的拦截
    // （症状：页面收到 204 + 弹窗下载）。静态服务器忽略 query，不影响加载。
    var sep = pdfUrl.indexOf('?') === -1 ? '?' : '&';
    iframe.src = resolveURL(pdfUrl + sep + 't=' + Date.now());
    iframe.className = 'ppv-frame';
    iframe.setAttribute('title', 'PDF 预览');
    // 注意：不能加 loading="lazy" —— Chrome 对 lazy iframe 中的 PDF 会触发下载
    // 而非内嵌显示（Chromium bug）。懒加载已由 IntersectionObserver 保证，
    // iframe 本身不再需要 lazy 属性。
    container.innerHTML = '';
    container.appendChild(iframe);
  }

  // ================================================================
  // pdf.js 完整 viewer 模式（默认）：iframe 加载同源 viewer.html，
  // viewer 内部同源 fetch PDF（无需 CORS），UI 接近浏览器原生。
  // ================================================================

  function loadViewer(container) {
    var pdfUrl = container.getAttribute('data-pdf-src');
    if (!pdfUrl || !CONFIG.viewerPath) {
      renderPdfjs(container, pdfUrl);
      return;
    }
    // 探测 viewer.html 是否部署（带 cache-buster 绕过首次请求拦截）
    var probeUrl = CONFIG.viewerPath + (CONFIG.viewerPath.indexOf('?') === -1 ? '?' : '&') + 't=' + Date.now();
    fetch(probeUrl, { method: 'HEAD' }).then(function (res) {
      if (!res.ok) {
        renderPdfjs(container, pdfUrl);
        return;
      }
      var iframe = document.createElement('iframe');
      // viewer.html?file=<绝对URL>&zoom=page-width&sidebar=1
      //   zoom=page-width：A4 页面宽度填满 viewer 内容区（高度按页面比例），解决"纸张没填满"
      //   sidebar=1：默认打开侧边栏（缩略图/大纲）
      // viewer 同源 fetch PDF，无需 CORS
      iframe.src = CONFIG.viewerPath + '?file=' + encodeURIComponent(resolveURL(pdfUrl)) +
        '&zoom=page-width&sidebar=1&t=' + Date.now();
      iframe.className = 'ppv-frame';
      iframe.setAttribute('title', 'PDF 预览');
      container.innerHTML = '';
      container.appendChild(iframe);
    }).catch(function () {
      renderPdfjs(container, pdfUrl);
    });
  }

  /** pdfjs Canvas 渲染（viewer 不可用时的 fallback） */
  function renderPdfjs(container, pdfUrl) {
    loadPDFJS(function () {
      var v = new PDFViewer(container, pdfUrl);
      instances.push(v);
      v.load();
    });
  }

  // ================================================================
  // 初始化
  // ================================================================

  function init() {
    var containers = document.querySelectorAll('.pdfviewer-container');
    if (containers.length === 0) return;
    applyTheme();

    // file:// 协议：用浏览器原生 <embed> 渲染（原版行为，Chrome 允许 embed 加载 file:// PDF）。
    // 若个别环境 embed 被拦截，用户仍可按提示改用 http 访问。
    if (isFileProtocol()) {
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

    // HTTP 协议：IntersectionObserver 视口自加载，按 data-preview-mode 分派
    // 默认 viewer（pdf.js 完整 viewer）；pdfjs/embed 为显式配置的备选
    function startRender(c) {
      if (c.getAttribute('data-ppv-ready')) return;
      c.setAttribute('data-ppv-ready', 'true');
      var pdfUrl = c.getAttribute('data-pdf-src');
      if (!pdfUrl) return;
      var mode = c.getAttribute('data-preview-mode') || 'viewer';
      if (mode === 'pdfjs') {
        renderPdfjs(c, pdfUrl);
      } else if (mode === 'embed') {
        // embed：iframe 内嵌，浏览器原生 PDF viewer（需服务器支持 CORS）
        loadNative(c);
      } else {
        // viewer（默认）：pdf.js 完整 viewer，viewer.html 缺失时自动 fallback 到 Canvas
        loadViewer(c);
      }
    }

    if ('IntersectionObserver' in window) {
      var observer = new IntersectionObserver(function (entries) {
        for (var i = 0; i < entries.length; i++) {
          if (entries[i].isIntersecting) {
            observer.unobserve(entries[i].target);
            startRender(entries[i].target);
          }
        }
      }, { rootMargin: '200px' });  // 提前 200px 预加载

      for (var i = 0; i < containers.length; i++) {
        observer.observe(containers[i]);
      }
    } else {
      // 不支持 IntersectionObserver 的浏览器：立即加载
      for (var i = 0; i < containers.length; i++) {
        startRender(containers[i]);
      }
    }
  }

  function observeTheme() {
    var observer = new MutationObserver(function () {
      applyTheme();
      // 更新 pdf.js Canvas 实例（embed 模式 iframe 由浏览器接管，无需处理）
      for (var i = 0; i < instances.length; i++) {
        var v = instances[i];
        if (v.canvas) v.canvas.style.background = getThemeColors().bg;
        var bar = v.container.querySelector('.ppv-toolbar');
        var wrap = v.container.querySelector('.ppv-canvas-wrapper');
        if (bar) bar.style.background = getThemeColors().canvas;
        if (wrap) wrap.style.background = getThemeColors().canvas;
      }
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function () { init(); observeTheme(); });
  } else {
    init(); observeTheme();
  }
})();
