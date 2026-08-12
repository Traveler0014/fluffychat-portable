# FluffyChat Web 离线单二进制打包 —— 方案评估

> 评估日期：2026-08-12 ｜ 评估对象：FluffyChat v2.8.0 (build 3558) Web 版
> 结论先行：**可行**，且已用真·断网环境端到端验证通过。推荐 **Tauri v2 + WebView2** 方案，备选 **Go embed** 方案。

---

## 1. 问题根源

- `fluffychat.im` 实际托管在 **GitHub Pages** 上（响应头 `server: GitHub.com`，边缘节点在德国 FRA，经新加坡 Varnish 缓存），国内访问极慢。
- 实测下载：`main.dart.js`（11MB）在 60s 内只下完 3MB（≈50KB/s）；GitHub release 直连仅 16KB/s。**应用本体加载是痛点所在**。

## 2. Web 应用结构与离线化可行性（已实测验证）

| 组件 | 大小 | 说明 |
|---|---|---|
| `main.dart.js` + 56 个 deferred chunk | ~13.5MB | Flutter dart2js 编译产物，按路由懒加载 |
| `canvaskit/` | 37MB(全量)/7MB(裁剪) | 渲染引擎 wasm；**注意默认从 gstatic CDN 拉取** |
| `assets/` | 3.2MB | 字体、图标、sounds、vodozemac(E2EE 加密 wasm) |
| 其它（flutter.js、Imaging.wasm、auth.html 等） | <1MB | OAuth/SSO 需要 auth.html |
| source map / skwasm / wimp / symbols | ~31MB | **可全部剔除**（调试与备选渲染器） |

### 离线化的 3 个关键改造点（均已验证）

1. **canvaskit 本地化**（必改）：Flutter loader 默认 `useLocalCanvasKit=false`，会从
   `https://www.gstatic.com/flutter-canvaskit/<revision>/chromium/canvaskit.wasm` 拉 5.7MB。
   实测 `load({config:{canvasKitBaseUrl:'canvaskit'}})` 对 v2.8.0 的 loader 无效，需**双保险**：
   - 在 `_flutter.buildConfig` 里注入 `useLocalCanvasKit: true`（该版本 loader 从这里读取此字段）
   - `load()` 调用处注入 `config: { canvasKitBaseUrl: 'canvaskit' }`
   ✅ 已验证：本地服务下 canvaskit 从 `/canvaskit/chromium/` 加载。

2. **`<base href="/web/">` 改为 `<base href="/">`**：否则本地起服务后所有相对路径错乱。✅ 已验证。

3. **禁用 Flutter 自带 Service Worker**（删除 index.html 中的 `serviceWorker: {...}` 块）：
   本地静态服务 + 免 SW 版本缓存问题。✅ 已验证（SW 加载失败 loader 本身也会静默降级）。

### 真·断网验证结果（拦截所有外部请求，Playwright 实测）

- ✅ Flutter app 完整启动：路由 `#/home`、`flt-glass-pane` 渲染、1280x720 canvas 绘制、UI 非白像素 97.3%
- ✅ 全部资源本地加载：`main.dart.js`、56 chunks、canvaskit、vodozemac E2EE wasm、字体图标
- ✅ 唯一外部请求 = 1 次 `fonts.gstatic.com` 字体（**Flutter 引擎的字体回退机制**，字符缺字形时按需拉取）。
  离线/拦截后**静默回退系统字体**，无报错、无白屏（中文走微软雅黑等系统字体，不影响功能）。
- ✅ `config.json` 请求 404 无害（线上同样 404，app 回退内置默认值）——但**提供自定义 config.json 反而是定制机会**（见 §6）。

## 3. 打包方案对比（win32 单二进制）

### 方案 A：Tauri v2 + WebView2（推荐 ⭐）

- 原理：把裁剪后的 web 资源嵌入 Tauri 前端，用系统 WebView2（Edge/Blink 内核）渲染，打包为**单 .exe**。
- 优点：
  - 真桌面应用：独立窗口、任务栏图标、系统托盘、**系统消息通知**（Matrix 消息提醒，Web 版在浏览器里做不到）
  - 与 Chrome 同内核，WebGL 支持好，canvaskit 走 chromium 变体无障碍
  - Windows 10/11 绝大多数自带 WebView2，exe 不含运行时，体积小（~20-30MB）
  - 官方对 Windows 无桌面版（release 只有 linux/web/apk），Tauri 是最接近"官方桌面体验"的路线
- 缺点：极老/精简 Windows 可能缺 WebView2（可附带静默安装器）；需装 Rust 工具链做首次开发
- 工作量：**1-2 天**（脚手架 + 资源嵌入 + 窗口/通知配置 + 打包）

### 方案 B：Go embed + 本地静态服务 + 默认浏览器（最简/最稳）

- 原理：web 资源 `go:embed` 进单二进制，运行时监听 `127.0.0.1:<随机空闲端口>`，自动打开默认浏览器。
- 优点：**零运行时依赖**（只要系统有 Chrome/Edge/Firefox），代码最少（~200 行），真·单文件，
  跨平台可复用同一套代码；Go 交叉编译 win32/x64 极其简单。
- 缺点：跑在浏览器标签页里（无独立窗口/托盘/系统通知）；多标签页共享 localhost 服务（无碍，单用户场景）；
  端口需自动避让。
- 工作量：**半天**。

### 方案 C：Electron / Nativefier（不推荐）

- 体积 80-200MB、内存占用高、非真单文件（需 NSIS 打包）。仅当需要完整 Chromium 内核时考虑，本场景无必要。

### 方案 D：直接编译 Flutter Windows 原生版（旁路，非"web 打包"）

- 官方无 Windows 桌面发布，需本地装 Flutter SDK + 各种 MSVC 依赖，产物是 exe+data 目录非单文件。
- 若未来上游正式支持 Windows 桌面可再考虑；当前 Web 打包路线更轻。

| 维度 | A: Tauri | B: Go embed | C: Electron | D: Flutter native |
|---|---|---|---|---|
| 单 exe | ✅ | ✅ | ⚠️(需打包器) | ❌(exe+data) |
| 体积 | 20-30MB | 25-35MB | 80-200MB | ~100MB |
| 桌面体验(窗口/托盘/通知) | 完整 | 无(浏览器) | 完整 | 完整 |
| 运行时依赖 | WebView2(系统自带为主) | 任意浏览器 | 无 | 无 |
| 开发工作量 | 1-2 天 | 半天 | 0.5 天 | 数天+工具链 |
| 升级维护 | 重新打包 | 重新打包 | 重新打包 | 重新构建 |

## 4. 推荐实施路径（方案 A 主推，B 兜底）

1. **裁剪构建**（脚本化，约 10 分钟/次）：
   - 从 GitHub release 下载 `fluffychat-web.tar.gz`（国内可用 `gh.ddlc.top` / `ghfast.top` 加速镜像，实测 460KB/s）
   - 剔除 `*.map`、`canvaskit/skwasm*`、`canvaskit/wimp*`、`canvaskit/*.symbols`（约省 31MB）；
     保留 `canvaskit/canvaskit.js/.wasm` + `canvaskit/chromium/`（无 WebGL 环境兜底）
   - 改写 index.html（base href、canvaskit 本地化、去 SW）
2. **Tauri v2 工程**：`src-tauri` 配置窗口标题/图标/固定 640x960 起始尺寸；`tauri.conf.json` 的
   `frontendDist` 指向裁剪产物；`bundle.targets: ["nsis"]` 出单文件安装包或 `["msi"]`；
   或 `--bundles nsis` 出免安装便携 exe。
3. **可选定制**：提供自定义 `config.json`（见 §5）预设 `defaultHomeserver` / `presetHomeserver` 为国内服务器，
   开箱即用。
4. **打包产物**：单 .exe（Tauri）/ 单 .exe（Go，无安装）。

## 5. 风险与注意事项（务必告知用户）

1. **Matrix 通信仍需联网**（核心约束）：单二进制只解决"界面加载慢"。消息同步走用户选择的 homeserver——
   若 homeserver 在境外，收发依然慢。**建议配合自建/国内 homeserver**，并可用 `config.json` 预设默认服务器。
2. **字体**：离线时引擎字体回退到系统字体（中文=微软雅黑），观感略有差异，功能无影响。
   若要求完全一致，可将引擎 `fontFallbackBaseUrl` 指到本地字体（增量工作）。
3. **E2EE 数据**：登录会话、密钥、消息缓存放浏览器/WebView2 的持久化存储（IndexedDB）。
   Tauri 方案数据在用户目录，卸载不影响；换机器需用 FluffyChat 的加密备份/恢复流程。
4. **上游更新**：FluffyChat 每月小版本，重新走一遍"下载 release → 裁剪 → 重打包"（脚本化后十几分钟）。
   `version.json`（2.8.0 build 3558）可用于展示版本号。
5. **canvaskit 裁剪权衡**：只留 chromium 变体最小 7MB；保留根目录 canvaskit.wasm 多 6.9MB 但兼容无 WebGL 环境
   （远程桌面/虚拟显卡）。win32 桌面目标建议保留双份。
6. **安全**：打包即分发 —— 建议只对可信用户分发；GitHub Pages 资源本身无 DRM 约束（开源 GPLv3）。

## 6. 附：config.json 定制机会

FluffyChat 支持从同目录 `config.json` 读取运行时配置（线上缺失则 404 回退默认）。`config.sample.json` 支持字段包括：

```json
{
  "applicationName": "FluffyChat",
  "defaultHomeserver": "matrix.org",        // ← 可改成你的国内服务器
  "presetHomeserver": "",                    // ← 登录页预设服务器（可填多个）
  "welcomeText": "",
  "renderHtml": true,
  "enableSoftLogout": false
}
```

打包时写入自定义 config.json 即可改变开箱行为，无需改代码。

## 7. 附：POC 验证产物（本机 /tmp/fcw）

- `bundle/` —— 裁剪后的 30MB 离线包（已本地化 canvaskit）
- `offline-test.mjs` —— Playwright 断网验证脚本（拦截外部请求，结果见 §2）
- `offline-shot.png` —— 离线运行截图（UI 渲染正常）

## 8. 下一步建议

1. 确认目标 Windows 版本分布（Win10/11？32/64 位？）→ 决定 A/B 方案与 canvast 裁剪粒度
2. 确认用户主要使用的 Matrix homeserver（境内？）→ 决定 config.json 定制
3. 若走 Tauri：本机装 Rust + 一次脚手架；若走 Go：直接开工（可先出 win32 x64 原型）
