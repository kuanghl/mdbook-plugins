/**
 * mdbook-pdf-preview — 嵌入式 PDF 预览运行时
 *
 * 由 mdbook-plugins pre-pdfviewer 预处理器配合使用。
 * 通过 book.toml 的 additional-js 加载。
 *
 * 功能：
 *   1. 检测 `.pdfviewer-container` 容器，自动渲染 PDF
 *   2. Canvas 直接渲染 PDF，支持翻页/缩放/关闭
 *   3. 主题跟随：自动适配 mdbook 主题（Light/Coal/Ayu/Catppuccin 等）
 *
 * pdf.js 加载策略（v3.11.174）：
 *   - 优先从 CDN 加载（Worker 正常）
 *   - CDN 加载失败时回退到本地 build/pdf.js（Worker 不可用，使用 fake worker）
 */
(function () {
  'use strict';

  // ================================================================
  // 配置
  // ================================================================
  var CONFIG = {
    // CDN 路径（主用，Worker 自动加载正常）
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
      CONFIG.localJsPath = baseUrl + '/build/pdf.min.mjs';
      CONFIG.localWorkerPath = baseUrl + '/build/pdf.worker.min.mjs';
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

  /** 尝试加载 pdf.js（本地 ESM 优先 → CDN ESM 回退） */
  function loadPDFJS(callback) {
    if (pdfjsReady) { callback(); return; }
    if (typeof pdfjsLib !== 'undefined' && pdfjsLib.GlobalWorkerOptions) {
      pdfjsReady = true;
      callback();
      return;
    }

    function showError() {
      var containers = document.querySelectorAll('.pdfviewer-container');
      for (var i = 0; i < containers.length; i++) {
        containers[i].innerHTML = '<div class="ppv-error">❌ PDF 库加载失败</div>';
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

  function getFilename(url) {
    return decodeURIComponent((url.replace(/[?#].*$/, '').split('/').pop() || url));
  }

  function escapeHtml(s) {
    var d = document.createElement('div');
    d.appendChild(document.createTextNode(s));
    return d.innerHTML;
  }

  // ================================================================
  // PDFViewer 实例
  // ================================================================

  function PDFViewer(container, pdfUrl) {
    this.container = container;
    this.pdfUrl = pdfUrl;
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
      var task = pdfjsLib.getDocument(self.pdfUrl);
      self.pdf = await task.promise;
      self.pageCount = self.pdf.numPages;
      self._buildUI();
      await self._renderPage();
    } catch (err) {
      self.container.innerHTML = '<div class="ppv-error">❌ ' + escapeHtml(err.message || '加载失败') + '</div>';
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
  // 初始化
  // ================================================================

  /** 检测是否为 file:// 协议（浏览器禁止 fetch/XHR 加载 PDF） */
  function isFileProtocol() {
    return window.location.protocol === 'file:';
  }

  /** 将相对路径解析为绝对路径 */
  function resolveURL(url) {
    var a = document.createElement('a');
    a.href = url;
    return a.href;
  }

  /** file:// 协议下降级方案：使用浏览器原生 <embed> 渲染 PDF */
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

  function init() {
    var containers = document.querySelectorAll('.pdfviewer-container');
    if (containers.length === 0) return;
    var colors = applyTheme();
    var isFile = isFileProtocol();

    // file:// 协议：浏览器原生渲染（立即执行）
    if (isFile) {
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

    // HTTP 协议：IntersectionObserver 视口自加载
    // 容器进入视口时才加载 pdf.js 并渲染，避免首屏加载 ~1.7MB 资源
    function startRender(c) {
      if (c.getAttribute('data-ppv-ready')) return;
      c.setAttribute('data-ppv-ready', 'true');
      var pdfUrl = c.getAttribute('data-pdf-src');
      if (!pdfUrl) return;
      loadPDFJS(function () {
        var v = new PDFViewer(c, pdfUrl);
        instances.push(v);
        v.load();
      });
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
      var c = applyTheme();
      // 更新 pdf.js Canvas 实例
      for (var i = 0; i < instances.length; i++) {
        var v = instances[i];
        if (v.canvas) v.canvas.style.background = c.bg;
        var bar = v.container.querySelector('.ppv-toolbar');
        var wrap = v.container.querySelector('.ppv-canvas-wrapper');
        if (bar) bar.style.background = c.canvas;
        if (wrap) wrap.style.background = c.canvas;
      }
      // 更新 file:// embed 容器
      var borderColor = c.canvas === '#e8e8e8' ? '#d0d0d0' : '#444';
      var bars = document.querySelectorAll('.ppv-embed-bar');
      for (var i = 0; i < bars.length; i++) {
        bars[i].style.background = c.canvas;
        bars[i].style.color = c.text;
        bars[i].style.borderBottom = '1px solid ' + borderColor;
        var icon = bars[i].querySelector('.ppv-embed-icon svg');
        if (icon) icon.style.fill = c.text;
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
