# SnapStudio 桌面版构建指南

跨平台桌面截图标注工具（Tauri 2 + 原生 Rust 能力层）。
同一份前端页面（`ui/index.html`）同时驱动浏览器版与桌面版。

## 项目结构

```
snap-studio-app/
├── package.json              # tauri CLI 脚本（无前端依赖，纯静态页面）
├── ui/index.html             # 前端（由 ../snap-studio/index.html + build-ui.js 注入桥接层生成）
├── build-ui.js               # 前端生成脚本（Web 版 → 桌面版桥接注入）
├── make_icons.py             # 图标生成脚本（Pillow）
└── src-tauri/
    ├── Cargo.toml            # Rust 依赖（tauri / xcap / global-shortcut / clipboard / rfd）
    ├── tauri.conf.json       # 窗口、托盘、打包、CSP 配置
    ├── capabilities/default.json
    ├── icons/                # 应用图标（png / ico / icns，已生成）
    └── src/
        ├── main.rs           # 入口（Windows 隐藏控制台）
        ├── lib.rs            # 插件装配 + 全局热键
        ├── capture.rs        # 截屏 + 多显示器定位（xcap）
        ├── commands.rs        # Tauri 命令（grab_screen / export_capture / reset_window）
        └── tray.rs           # 系统托盘
```

## Windows 本机构建（一次配置，之后秒级增量）

### 1. 安装工具链（仅需一次）

```powershell
# Rust（含 MSVC 后端所需组件）
winget install Rustlang.Rustup
rustup default stable

# VS Build Tools 的 C++ 编译工具（会弹 UAC，勾选「使用 C++ 的桌面开发」）
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

> 已有 Node ≥ 18 即可（`node --version` 验证）。

### 2. 构建

```bash
cd snap-studio-app
npm install
npm run tauri build
```

产物在 `src-tauri/target/release/bundle/` 下：

- `nsis/*.exe` —— NSIS 安装包（推荐分发这个）
- `msi/*.msi`  —— MSI 安装包

### 3. 开发调试

```bash
npm run tauri dev
```

热键 `Ctrl+Shift+A` 全局可用（任何界面下）。

## macOS 构建（在 macOS 机器或 GitHub Actions 上）

macOS 上两个摩擦点：

1. **屏幕录制权限**：首次截屏时 macOS 会弹窗要求授权
   「系统设置 → 隐私与安全性 → 屏幕录制」勾选 SnapStudio，然后重启应用。
   未授权时抓到的画面全黑，应用会主动提示这一点。
2. **Gatekeeper**：未签名的 App 首次打开需右键 → 打开（或
   `xattr -cr /Applications/SnapStudio.app`）。正式分发需要 Apple Developer 签名 + 公证。

### 本机构建（Intel / Apple Silicon 均可）

```bash
cd snap-studio-app
rustup target add x86_64-apple-darwin aarch64-apple-darwin
npm install
npm run tauri build -- --target universal-apple-darwin
```

产物在 `src-tauri/target/universal-apple-darwin/release/bundle/` 下：

- `dmg/*.dmg` —— 磁盘镜像（推荐分发这个）
- `macos/*.app` —— 裸应用包

### GitHub Actions 云端构建（推荐：无需本地 macOS）

把整个 `snap-studio-app/` 推到 GitHub 仓库后，Actions 会自动构建：
`.github/workflows/build.yml` 会产出 Windows x64 安装包 + macOS universal dmg。
在仓库 Actions 页面下载对应平台的产物即可。

手动触发：进入 Actions → Build SnapStudio → Run workflow。

## 前端修改流程

桌面版前端由脚本从 Web 版生成，**改 Web 版，不改 ui/index.html**：

```bash
# 1. 修改 ../snap-studio/index.html（Web 版源文件）
# 2. 重新生成桌面版前端
node build-ui.js
# 3. 重新构建
npm run tauri build
```

`build-ui.js` 注入的内容：Tauri 环境检测（IS_TAURI）、原生截屏/复制/保存分支、
全屏截图态 CSS、Esc 退出逻辑、无边框窗口拖拽区。所有标注、框选、放大镜逻辑
两侧完全共用。

## 关键设计说明

- **热键**：`Ctrl+Shift+A`（Windows）/ `Cmd+Shift+A`（macOS），
  由 `tauri-plugin-global-shortcut` 注册，任何应用在前台时都能触发。
- **截屏定位**：光标所在显示器整屏截取（xcap），主窗口随即铺满该显示器，
  在其上框选标注。多显示器用物理坐标 → 逻辑坐标换算对齐。
- **导出**：合成底图 + 标注层为 PNG 后，走系统剪贴板（`write_image`）
  或保存对话框（rfd）。
- **CSP**：`script-src 'unsafe-inline'` 允许内联脚本（桥接层直接写在页面里）；
  `withGlobalTauri: true` 注入 `window.__TAURI__` 全局 API。
