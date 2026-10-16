// 由 Web 版 index.html 生成 Tauri 版 ui/index.html（注入桥接层）
const fs = require('fs');
const path = require('path');

const SRC = path.join(__dirname, '..', 'snap-studio', 'index.html');
const OUT = path.join(__dirname, 'ui', 'index.html');

let html = fs.readFileSync(SRC, 'utf8');

const CAPTURING_CSS = `
/* ===== Tauri 截屏全屏态 ===== */
body.capturing .app{max-width:none;padding:0;gap:0}
body.capturing .topbar,body.capturing .hints{display:none}
body.capturing .stage{border:none;border-radius:0;padding:0;min-height:100vh}
body.capturing .boardwrap{position:fixed;inset:0;display:block!important;border-radius:0;box-shadow:none;background:#000}
body.capturing #displayCanvas{max-width:100vw!important;max-height:100vh!important;width:auto!important;height:auto!important;border-radius:0}
body.capturing #selBox{box-shadow:0 0 0 9999px rgba(4,6,14,.62)}
/* ===== Tauri 长截图浮动控制条态（主窗缩为小条，须置于 capturing 规则之后） ===== */
body.longmode{overflow:hidden}
body.longmode .boardwrap{display:none!important}
body.longmode .stage,body.longmode .app{min-height:0!important}
body.longmode #longBar{left:0;right:0;top:0;bottom:0;transform:none;border-radius:0;justify-content:center;padding:0 12px;gap:10px;background:rgba(10,13,22,.92)}
body.longmode #toast{bottom:auto;top:6px}
`;

// 1) 注入截屏全屏态 CSS
html = html.replace('</style>', CAPTURING_CSS + '</style>');

// 2) 顶栏可拖动（Tauri 无边框窗口）
html = html.replace('<header class="topbar">', '<header class="topbar" data-tauri-drag-region>');

// 3) IS_TAURI 标志
html = html.replace("'use strict';", "'use strict';\n  var IS_TAURI = !!(window.__TAURI__ && window.__TAURI__.core);");

// 4) 捕获/复制/保存 分支走原生
html = html.replace('function captureScreen() {', 'function captureScreen() { if (IS_TAURI) { startTauriCapture(); return; }');
html = html.replace('function doCopy() {', "function doCopy() { if (IS_TAURI) { tauriExport('copy'); return; }");
html = html.replace('function doSave() {', "function doSave() { if (IS_TAURI) { tauriExport('save'); return; }");

// 5) Esc 在全屏态退出截屏
html = html.replace(
  "if (S.sel) { resetSelection(); setStatus('已取消选区', false); }",
  "if (IS_TAURI && document.body.classList.contains('capturing')) { exitCaptureMode(); return; }\n      if (S.sel) { resetSelection(); setStatus('已取消选区', false); }"
);

// 6) 取消按钮在全屏态退出截屏
html = html.replace(
  "    resetSelection();\n    setStatus('已取消选区', false);\n  });",
  "    if (IS_TAURI && document.body.classList.contains('capturing')) { exitCaptureMode(); return; }\n    resetSelection();\n    setStatus('已取消选区', false);\n  });"
);

// 7) 注入桥接代码（IIFE 内，可访问内部函数）
const BRIDGE = `
  // ============ Tauri 原生桥接 ============
  var T = IS_TAURI ? window.__TAURI__ : null;

  function startTauriCapture() {
    var scope = 'screen';
    var sel = document.getElementById('scopeSel');
    if (sel) scope = sel.value || 'screen';
    var cmd = scope === 'all' ? 'grab_all_screens' : (scope === 'window' ? 'grab_window' : 'grab_screen');
    var label = scope === 'all' ? '全部显示器' : (scope === 'window' ? '前台窗口' : '屏幕');
    toast('正在捕获' + label + '…');
    T.core.invoke(cmd).catch(function (e) { toast('捕获失败：' + e); });
  }

  function enterCaptureMode(shot) {
    // 热键打断长截图：安静清理会话（不碰窗口几何，随后 Rust 会铺满覆盖层）
    if (S.long) {
      var wasAuto = S.long.auto;
      S.long = null;
      hideLongBar();
      document.body.classList.remove('longmode');
      toolbar.style.display = '';
      if (wasAuto) {
        try { window.__tauri__.endAutoScroll().catch(function () {}); } catch (e) {}
      }
    }
    document.body.classList.add('capturing');
    S.capture = shot;   // 记录物理原点，供长截图把选区换算成屏幕物理坐标
    var img = new Image();
    img.onload = function () {
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(img, 0, 0);
      S.img = canvas;
      emptyEl.classList.add('hidden');
      board.style.display = 'block';
      canvas.style.maxWidth = '100vw';
      canvas.style.maxHeight = '100vh';
      resetSelection();
      setStatus('拖拽框选目标区域 · ' + shot.monitor, true);
      requestAnimationFrame(function () {
        try { T.window.getCurrentWindow().show(); T.window.getCurrentWindow().setFocus(); } catch (e) {}
      });
    };
    img.onerror = function () { toast('画面加载失败'); };
    img.src = shot.dataUrl;
  }

  function exitCaptureMode() {
    document.body.classList.remove('capturing');
    resetSelection();
    setStatus('待机中', false);
    // 用完即走：复位防重入标志 + 恢复几何并隐藏回托盘，不再弹出主窗
    try { T.core.invoke('finish_capture').catch(function(){}); } catch (e) {}
  }

  function tauriExport(action) {
    if (!S.sel) { toast('请先框选区域'); return; }
    var c = compose();
    var dataUrl = c.toDataURL('image/png');
    var b64 = (dataUrl.split(',')[1]) || '';
    toast('正在导出…');
    T.core.invoke('export_capture', { pngBase64: b64, action: action })
      .then(function () {
        if (action === 'copy') toast('已复制到剪贴板 · ' + c.width + '×' + c.height);
        else toast('已保存 PNG · ' + c.width + '×' + c.height);
        exitCaptureMode();
      })
      .catch(function (e) { toast('导出失败：' + e); });
  }

  if (IS_TAURI) {
    // 暴露原生专属能力给前端（网页版无此对象，自动回退）
    window.__tauri__ = {
      saveImageToFolder: function (bytes, name) {
        return T.core.invoke('save_image_to_folder', { bytes: bytes, name: name });
      },
      pinImage: function (bytes) {
        return T.core.invoke('pin_image', { bytes: bytes });
      },
      captureRegion: function (x, y, w, h) {
        return T.core.invoke('capture_region', { x: x, y: y, w: w, h: h });
      },
      enterLongMode: function (w, h, cx, cy) {
        return T.core.invoke('enter_long_mode', { w: w, h: h, cx: cx, cy: cy });
      },
      scrollAt: function (x, y, notches) {
        return T.core.invoke('scroll_at', { x: x, y: y, notches: notches });
      },
      endAutoScroll: function () {
        return T.core.invoke('end_auto_scroll');
      },
      resetWindow: function () {
        return T.core.invoke('reset_window');
      },
      setCaptureMode: function (mode) {
        return T.core.invoke('set_capture_mode', { mode: mode });
      }
    };

    // 「范围」选择器变化时同步截屏模式，供全局热键分流
    var scopeSelEl = document.getElementById('scopeSel');
    if (scopeSelEl) scopeSelEl.addEventListener('change', function () {
      T.core.invoke('set_capture_mode', { mode: this.value }).catch(function () {});
    });

    T.event.listen('snap:capture', function (ev) { enterCaptureMode(ev.payload); });
    setStatus('待机中 · 热键 Ctrl/⌘+Shift+A 截图', false);
    // 隐藏 Web 专属的“屏幕捕获”文案，桌面端用系统热键
    var capBtns = document.querySelectorAll('#btnCapture,#btnCapture2');
    Array.prototype.forEach.call(capBtns, function (b) { b.textContent = b.textContent.replace('捕获屏幕', '截屏 (热键)').replace('开始捕获', '截屏 (热键)'); });
  }
`;

html = html.replace('})();', BRIDGE + '\n})();');

fs.writeFileSync(OUT, html, 'utf8');
console.log('ui/index.html written, bytes =', html.length);
