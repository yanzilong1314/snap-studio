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

// 5.1) 悬停智能选区（类企业微信）：空闲移动时做元素级检测，绿框预选
html = html.replace(
  "overlay.style.cursor = (S.sel && insideSel(p)) ? (S.tool === 'text' ? 'text' : 'crosshair') : 'crosshair';",
  "overlay.style.cursor = (S.sel && insideSel(p)) ? (S.tool === 'text' ? 'text' : 'crosshair') : 'crosshair';\n      if (IS_TAURI) tauriHover(p);"
);

// 5.2) 单击（非拖拽）直接选中悬停检测到的区域
html = html.replace(
  "  function onUp(e) {\n    if (S.dragMode === 'select' && S.drag) {",
  "  function onUp(e) {\n    if (IS_TAURI && document.body.classList.contains('capturing') && S.dragMode === 'select' && S.drag) {\n      var _d = S.drag;\n      if (Math.abs(_d.x1 - _d.x0) < 6 && Math.abs(_d.y1 - _d.y0) < 6 && applyHoverSelection()) {\n        S.dragMode = null; S.drag = null; return;\n      }\n    }\n    if (S.dragMode === 'select' && S.drag) {"
);

// 5.3) 开始拖拽框选时隐藏绿框
html = html.replace(
  "  function updateSelFromDrag() {",
  "  function updateSelFromDrag() {\n    if (IS_TAURI) hideHover();"
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
    resetHover();
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
    resetHover();
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

  // ===== 悬停智能选区（类企业微信）：移到哪亮到哪，单击即选中 =====
  var HOVER = { box: null, pending: false, seq: 0, rect: null, lastMove: 0 };

  function hoverBox() {
    if (!HOVER.box) HOVER.box = document.getElementById('hoverBox');
    return HOVER.box;
  }
  // 仅隐藏绿框（保留 HOVER.rect：按下后 updateSelFromDrag 会先隐藏绿框，
  // 抬起时单击仍需用该矩形完成「点一下即选中」）
  function hideHover() {
    var b = HOVER.box;
    if (b) b.style.display = 'none';
  }
  // 退出/进入截屏态时彻底复位
  function resetHover() {
    HOVER.rect = null;
    hideHover();
  }
  // 空闲移动（未按下/未拖拽）时节流调用后端检测（节流 90ms + 在途去重）
  function tauriHover(p) {
    if (!T || !S.img) return;
    if (S.sel && insideSel(p)) { hideHover(); return; }
    var now = Date.now();
    if (HOVER.pending || now - HOVER.lastMove < 90) return;
    HOVER.lastMove = now;
    HOVER.pending = true;
    var seq = ++HOVER.seq;
    T.core.invoke('detect_hover_region').then(function (r) {
      HOVER.pending = false;
      if (seq !== HOVER.seq) return; // 过期结果丢弃
      showHoverRect(r);
    }).catch(function () { HOVER.pending = false; });
  }
  // 物理屏幕坐标 → 图像像素坐标（截图即该屏物理像素快照）→ 定位绿框
  function showHoverRect(r) {
    if (S.dragMode) return; // 拖拽中不打扰（含按下到抬起之间，保留按下前的矩形供单击选中）
    var b = hoverBox();
    if (!b) return;
    if (!r || !S.capture) { resetHover(); return; }
    var ix = r.x - S.capture.x, iy = r.y - S.capture.y;
    var x0 = clamp(ix, 0, canvas.width), y0 = clamp(iy, 0, canvas.height);
    var x1 = clamp(ix + r.w, 0, canvas.width), y1 = clamp(iy + r.h, 0, canvas.height);
    if (x1 - x0 < 4 || y1 - y0 < 4) { resetHover(); return; }
    HOVER.rect = { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
    var k = ratio();
    var bb = board.getBoundingClientRect();
    var rc = canvas.getBoundingClientRect();
    b.style.display = 'block';
    b.style.left = (rc.left - bb.left + x0 * k) + 'px';
    b.style.top = (rc.top - bb.top + y0 * k) + 'px';
    b.style.width = ((x1 - x0) * k) + 'px';
    b.style.height = ((y1 - y0) * k) + 'px';
  }
  // 单击选中悬停区域：把绿框转正为选区（成功返回 true）
  function applyHoverSelection() {
    var r = HOVER.rect;
    resetHover();
    if (!r || !S.img) return false;
    S.sel = { x: r.x, y: r.y, w: r.w, h: r.h };
    selBox.style.display = 'block';
    var k = ratio();
    var bb = board.getBoundingClientRect();
    var rc = canvas.getBoundingClientRect();
    selBox.style.left = (rc.left - bb.left + r.x * k) + 'px';
    selBox.style.top = (rc.top - bb.top + r.y * k) + 'px';
    selBox.style.width = (r.w * k) + 'px';
    selBox.style.height = (r.h * k) + 'px';
    sizeBadge.textContent = Math.round(r.w) + ' × ' + Math.round(r.h);
    return finalizeSelection();
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
