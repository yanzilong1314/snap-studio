# SnapStudio

一站式截图工作台（Tauri 2 + 纯前端编辑器）：截屏 → 标注 → 智能工具 → 钉图 → 输出。
同一份 Web 源码同时驱动桌面版（Windows / macOS）与浏览器网页版。

## 功能

| 分类 | 功能 |
|---|---|
| 截屏 | 当前屏幕 / 全部显示器拼接 / 前台窗口自动识别、延时截屏（0/3/5/10s） |
| 标注 | 矩形、椭圆、箭头、直线、画笔、荧光笔、马赛克、文字、序号；选中后移动/改色/删除；撤销重做（快照式） |
| 智能工具 | 本地离线 OCR（中英，tesseract.js）、翻译（MyMemory）、二维码识别与生成（jsQR / qrcodejs）、GIF 录屏（gif.js，≤15s） |
| 长截图 | 框选区域 → 浮动控制条 → 自动滚动+自动捕获+像素匹配去重拼接（或手动逐屏捕获） |
| 钉图 | 置顶透明小窗贴桌面，可拖动/双击关闭，托盘一键清除（最多 16 张） |
| 输出 | 复制剪贴板、保存到 `图片/SnapStudio`、浏览器下载 |

- 全局热键 `Ctrl/⌘+Shift+A` 发起截屏；托盘常驻。
- macOS「前台窗口识别」「自动滚动」需在 系统设置 → 隐私与安全性 → 辅助功能 授权。

## 目录结构

```
snap-studio/          Web 唯一真源（index.html），网页版直接使用
snap-studio-app/      Tauri 工程
  build-ui.js         从 Web 真源生成 ui/index.html（注入 Tauri 桥）
  ui/                 生成物 + pin.html（钉图窗口页）
  src-tauri/          Rust：xcap 抓屏、多屏拼接、区域裁剪、前台窗口识别、
                      自动滚轮注入、钉图窗口、托盘、全局热键、保存到文件夹
  .github/workflows/  CI：Windows x64 (nsis+msi) 与 macOS universal (dmg)
```

## 本地开发

```bash
cd snap-studio-app
npm install
node build-ui.js      # 修改 snap-studio/index.html 后重新生成 ui
npm run tauri dev
```

> 改前端只改 `snap-studio/index.html`（真源），再跑 `build-ui.js` 同步；不要直接改 `ui/index.html`。

## 构建与发布

推送到 `main` 触发 GitHub Actions 双平台构建，产物在 run 的 Artifacts：
`SnapStudio-windows-x64`（exe/msi）、`SnapStudio-macos-universal`（dmg）。

发版流程：

```bash
# 1. 升版本号（三处保持一致）
#    src-tauri/tauri.conf.json / src-tauri/Cargo.toml / package.json
# 2. 提交推送，等 CI 绿
git tag v0.2.0 && git push origin v0.2.0
gh release create v0.2.0 --title "SnapStudio v0.2.0" --notes "..."
```

## 关键实现备忘

- Tauri 2 `WebviewUrl` 只有 `External` / `App` / `CustomProtocol` 变体；钉图窗口用
  `WebviewUrl::App("pin.html")` + `get_pin_image` 命令取图（存 `PinImages` 状态，取走即删）。
- 自有命令（`invoke_handler` 注册）默认对所有窗口放行，capability 只需管插件权限；
  钉图标签池 `pin-0..pin-15` 在 `capabilities/pins.json` 单独授权窗口级权限。
- 截屏覆盖层与多屏拼接定位使用**物理坐标**（`PhysicalPosition/PhysicalSize`），
  混合 DPI 多屏下精确对位；长截图逐帧区域裁剪用负偏移 `imageops::overlay` 实现。
- 自动滚动：Windows `SetCursorPos` + `mouse_event(MOUSEEVENTF_WHEEL)`（预检
  `WindowFromPoint` 排除任务栏/桌面）；macOS osascript Page Down。
- 长截图拼接算法 `findScrollOffset`：逐行像素匹配求滚动量，纯函数已单测（8 用例）。
