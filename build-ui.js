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
body.capturing #displayCanvas{max-width:100vw!important;max-height:100vh!important;width:100vw!important;height:100vh!important;border-radius:0}
body.capturing #selBox{box-shadow:0 0 0 9999px rgba(4,6,14,.62)}
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
    toast('正在捕获屏幕…');
    T.core.invoke('grab_screen').catch(function (e) { toast('捕获失败：' + e); });
  }

  function enterCaptureMode(shot) {
    document.body.classList.add('capturing');
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
    try { T.core.invoke('reset_window').catch(function(){}); } catch (e) {}
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
