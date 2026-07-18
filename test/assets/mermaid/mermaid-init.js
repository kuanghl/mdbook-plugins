/**
 * Mermaid 初始化 + 浮动工具栏 + 点击放大
 */

(function() {
  var THEMES = ['Auto','Light','Forest','Ocean','Sunset','Dark','Dracula','Monokai','Nord','Original'];

  // ===== 浮动工具栏 =====
  function addToolbar(container) {
    if (container.querySelector('.mm-toolbar')) return;

    var toolbar = document.createElement('div');
    toolbar.className = 'mm-toolbar';

    var btn = document.createElement('button');
    btn.className = 'mm-theme-btn';
    btn.title = '切换此图主题';

    function updateBtn() {
      btn.textContent = container.getAttribute('data-mermaid-theme') || 'Auto';
    }

    btn.addEventListener('click', function(e) {
      e.stopPropagation();
      var current = container.getAttribute('data-mermaid-theme') || 'Auto';
      var idx = THEMES.indexOf(current);
      var next = THEMES[(idx + 1) % THEMES.length];
      if (next === 'Auto') container.removeAttribute('data-mermaid-theme');
      else container.setAttribute('data-mermaid-theme', next);
      updateBtn();
    });

    updateBtn();
    toolbar.appendChild(btn);
    container.appendChild(toolbar);
  }

  // ===== 点击放大（modal） =====
  function addZoom(svg, container) {
    svg.style.cursor = 'zoom-in';
    svg.addEventListener('click', function(e) {
      if (e.target.closest('.mm-toolbar')) return;
      openModal(svg);
    });
  }

  function openModal(svg) {
    var old = document.getElementById('mm_modal');
    if (old) old.remove();

    // 从最近的 .mermaid-container 继承主题属性
    var origContainer = svg.closest('.mermaid-container');
    var themeAttr = origContainer ? origContainer.getAttribute('data-mermaid-theme') : '';

    var wrapper = document.createElement('div');
    wrapper.className = 'mermaid-container';
    if (themeAttr) { wrapper.setAttribute('data-mermaid-theme', themeAttr); }
    wrapper.style.cssText = 'display:flex;justify-content:center;align-items:center;padding:20px;';

    var clone = svg.cloneNode(true);
    clone.style.maxWidth = '95vw';
    clone.style.maxHeight = '95vh';
    clone.style.cursor = 'grab';
    wrapper.appendChild(clone);

    var modal = document.createElement('div');
    modal.style.cssText = 'display:flex;position:fixed;z-index:9999;left:0;top:0;width:100%;height:100%;background:rgba(0,0,0,0.85);justify-content:center;align-items:center;';

    var close = document.createElement('span');
    close.textContent = '\u00D7';
    close.style.cssText = 'position:fixed;top:15px;right:35px;color:#fff;font-size:40px;cursor:pointer;';
    close.onclick = function() { modal.remove(); };
    modal.onclick = function(e) { if (e.target === modal) modal.remove(); };

    modal.appendChild(close);
    modal.appendChild(wrapper);
    document.body.appendChild(modal);

    // 拖动
    var isDown = false, sx, sy, tx = 0, ty = 0;
    clone.onmousedown = function(e) {
      isDown = true; sx = e.clientX - tx; sy = e.clientY - ty;
      clone.style.cursor = 'grabbing';
    };
    document.onmousemove = function(e) {
      if (!isDown) return;
      tx = e.clientX - sx; ty = e.clientY - sy;
      clone.style.transform = 'translate(' + tx + 'px,' + ty + 'px)';
    };
    document.onmouseup = function() {
      isDown = false; clone.style.cursor = 'grab';
    };

    // 滚轮缩放
    var scale = 1;
    wrapper.onwheel = function(e) {
      e.preventDefault();
      scale *= e.deltaY < 0 ? 1.1 : 0.9;
      scale = Math.min(Math.max(scale, 0.2), 5);
      clone.style.transform = 'translate(' + tx + 'px,' + ty + 'px) scale(' + scale + ')';
    };
  }

  // ===== 初始化 =====
  function init() {
    document.querySelectorAll('.mermaid-container').forEach(function(container) {
      addToolbar(container);
      var svg = container.querySelector('.mermaid svg');
      if (svg && !svg._mmZoom) { svg._mmZoom = true; addZoom(svg, container); }
    });
  }

  // 监听 mermaid 渲染完成（mermaid 渲染后会在 .mermaid 中插入 svg）
  var renderTimer = setInterval(function() {
    var pending = document.querySelectorAll('.mermaid:not(.mm-ready)');
    if (pending.length === 0) { clearInterval(renderTimer); return; }
    pending.forEach(function(el) {
      if (el.querySelector('svg')) {
        el.classList.add('mm-ready');
      }
    });
    init();
  }, 100);

  // 也监听 DOM 变化（动态插入的 mermaid 图）
  var obs = new MutationObserver(function() { init(); });
  document.addEventListener('DOMContentLoaded', function() {
    obs.observe(document.body, { childList: true, subtree: true });
  });

  mermaid.initialize({ startOnLoad: true });
})();
